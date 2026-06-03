//! Typed HTTP client over `reqwest`.
//!
//! Every CLI command that talks to als-api funnels through this
//! `Client`. The transport surface is intentionally small: five typed
//! helpers (`get_json`, `post_json`, `patch_json`, `delete`,
//! `post_multipart`) plus one form-encoded unauthenticated variant
//! used by the RFC 8628 device-flow endpoints. Endpoint modules wrap
//! these helpers with domain types and live in their own files.
//!
//! Response handling is layered:
//!
//! 1. Transport failures become [`ApiError::Network`].
//! 2. 204 / empty-body success responses short-circuit before any JSON
//!    parsing.
//! 3. Everything else goes through [`crate::envelope::parse_envelope`].
//! 4. `ApiError::RateLimited` is patched with the actual `Retry-After`
//!    header value before being returned to the caller, since the
//!    header lives outside the JSON body.
//!
//! Path semantics:
//!
//! * `Client::new` normalises the base URL so its path ends with `/api/`.
//!   Users may configure `https://api.a.ls`, `https://api.a.ls/api`,
//!   or `https://api.a.ls/api/` — all resolve to the same effective base.
//! * Endpoint modules pass *relative* paths (`"sites"`,
//!   `format!("sites/{id}")`). Leading slashes would clobber the `/api/`
//!   prefix because `Url::join` treats `/...` as absolute-path replacement.
//!
//! Transport selection:
//!
//! * The HTTP/1.1+2 client (`h2`) is built eagerly in [`Client::new`]
//!   and is always available as the fallback. Despite the name it
//!   negotiates h1 or h2 via TLS ALPN — `h2` here just means
//!   "non-h3 reqwest client".
//! * The decision is driven by `[transport."<host:port>"]` entries in
//!   `config.toml` (see `als_core::config::TransportEntry`) plus the
//!   `ALS_TRANSPORT=h3|h2|auto` env override, which always wins.
//!   Recognised `protocol` values:
//!   - `"h3"`  — **force** HTTP/3. No probe, no fallback. If h3 is
//!     unreachable the request surfaces the network error to the
//!     caller; that's the price of pinning.
//!   - `"h2"`  — **force** HTTP/1.1+2. No probe.
//!   - `"auto"` — probe each session; use h3 if the QUIC handshake
//!     succeeds, otherwise fall back to h2. The decision is
//!     in-memory only — never written to the config file.
//!   - missing entry — same as `"auto"`.
//! * The CLI never mutates the `[transport.*]` section. Users opt into
//!   `"h3"` / `"h2"` pins by editing `config.toml` themselves.
//! * Env override `ALS_TRANSPORT` follows the same value vocabulary
//!   but is runtime-only.

#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use reqwest::{Client as HttpClient, RequestBuilder, Response, StatusCode, multipart};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::OnceCell;
use url::Url;

use crate::envelope::{ListPayload, parse_envelope, parse_envelope_no_data, parse_list_envelope};
use crate::error::ApiError;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// HTTP/3 probe budget. Kept short so a blocked-UDP network does not
/// stall every CLI invocation by `DEFAULT_CONNECT_TIMEOUT` seconds before
/// falling back to HTTP/2.
const HTTP3_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const USER_AGENT: &str = concat!("als/", env!("CARGO_PKG_VERSION"));

/// Runtime override for the transport pin. Recognised values mirror the
/// `[transport."<host:port>"]` config entry: `"h3"`, `"h2"`, `"auto"`.
/// Anything else is treated as `"auto"`. Always wins over the config
/// file; never writes back.
const TRANSPORT_ENV: &str = "ALS_TRANSPORT";

const PROTOCOL_H3: &str = "h3";
const PROTOCOL_H2: &str = "h2";

fn env_transport() -> Option<String> {
    std::env::var(TRANSPORT_ENV)
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
}

/// Canonicalise the API base into the `host:port` key used by
/// `[transport."<key>"]`. `https://api.a.ls`, `https://api.a.ls/api/`,
/// and `https://api.a.ls/anything-else` all collapse to the same key
/// because the transport pin is about the *server*, not the path.
fn host_port_key(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    Some(format!("{host}:{port}"))
}

fn build_h2() -> Result<HttpClient, ApiError> {
    Ok(HttpClient::builder()
        .user_agent(USER_AGENT)
        .timeout(DEFAULT_TIMEOUT)
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .build()?)
}

/// HTTP/3 prior-knowledge client — every request through it is sent
/// over QUIC. A build failure (e.g. no usable rustls provider, or
/// being called outside a tokio runtime so quinn can't set up its UDP
/// socket) is *not* fatal: we just return `None` and let the caller
/// fall back to HTTP/2.
///
/// Must be invoked from inside a tokio runtime: `quinn::Endpoint`
/// calls `tokio::runtime::Handle::current()` during construction.
fn build_h3() -> Option<HttpClient> {
    HttpClient::builder()
        .user_agent(USER_AGENT)
        .timeout(DEFAULT_TIMEOUT)
        .connect_timeout(HTTP3_CONNECT_TIMEOUT)
        .http3_prior_knowledge()
        .build()
        .ok()
}

/// Probe an HTTP/3 client by issuing a HEAD against the API base. Any
/// HTTP-level response (even 404 / 405) means the QUIC handshake
/// succeeded, which is all we need to commit to HTTP/3. Only transport
/// errors (UDP blocked, certificate failure, timeout) flip us to the
/// HTTP/2 fallback.
async fn probe_h3(h3: &HttpClient, base: &Url) -> bool {
    h3.head(base.clone()).send().await.is_ok()
}

/// Normalise a base URL so its path ends with `/api/`. Accepts the three
/// reasonable forms callers might write in `config.toml`:
///
/// * `https://api.a.ls`           → `https://api.a.ls/api/`
/// * `https://api.a.ls/api`       → `https://api.a.ls/api/`
/// * `https://api.a.ls/api/`      → `https://api.a.ls/api/`
///
/// Anything else with a non-`/api` path (e.g. `https://x/v1`) is left
/// alone except for a trailing-slash fixup; advanced users can opt out
/// of the auto-prefix by writing a path that already contains `api`
/// somewhere in it.
fn normalize_base(mut url: Url) -> Url {
    let path = url.path();
    let new_path = if path.contains("/api") {
        if path.ends_with('/') {
            path.to_owned()
        } else {
            format!("{path}/")
        }
    } else if path.ends_with('/') {
        format!("{path}api/")
    } else {
        format!("{path}/api/")
    };
    url.set_path(&new_path);
    url
}

/// Authenticated HTTP client for the als-api server.
///
/// Constructed once per CLI invocation from a `als_core::Config` and
/// then passed by reference to each endpoint function. The contained
/// `SecretString` keeps the API token out of `Debug` output and zeros
/// the buffer on drop.
pub struct Client {
    /// HTTP/1.1 + HTTP/2 client. Always present; serves every request
    /// once transport resolution settles on anything other than h3.
    h2: HttpClient,
    /// Resolved transport. Lazily populated on the first async call
    /// because `quinn::Endpoint` (used by reqwest's HTTP/3 backend)
    /// requires a live tokio runtime — `Client::new` is sync and may
    /// be called before a runtime exists.
    transport: OnceCell<HttpClient>,
    base: Url,
    bearer: SecretString,
    /// Path to the config file we *read* the `[transport.*]` pin from.
    /// `None` when no path can be resolved (e.g. unauthenticated
    /// client with a broken XDG env); resolution still works, it just
    /// never sees a file pin and falls back to `"auto"` semantics.
    config_path: Option<PathBuf>,
}

impl Client {
    /// Build a client from the resolved CLI config.
    ///
    /// * rustls TLS (workspace default).
    /// * `User-Agent: als/<crate-version>`.
    /// * 30 s overall request timeout, 10 s connect timeout on the
    ///   HTTP/2 client; the HTTP/3 client uses a tighter 3 s connect
    ///   budget so a blocked-UDP network falls back quickly.
    pub fn new(cfg: &als_core::Config) -> Result<Self, ApiError> {
        Ok(Self {
            h2: build_h2()?,
            transport: OnceCell::new(),
            base: normalize_base(cfg.api.clone()),
            bearer: cfg.token.clone(),
            config_path: Some(cfg.source_path.clone()),
        })
    }

    /// Build a client with the same transport settings as [`Self::new`]
    /// but no bearer token. Used by `als auth login`, which must call
    /// the unauthenticated device-flow endpoints before any token
    /// exists.
    ///
    /// The same `http`/`https` scheme guard the authenticated builder
    /// relies on `als_core::config` for is enforced inline here so a
    /// caller that constructs `Url` directly (bypassing config load)
    /// can't reach a `file://` or `data:` target.
    pub fn unauthenticated(api: Url) -> Result<Self, ApiError> {
        match api.scheme() {
            "http" | "https" => {}
            other => {
                return Err(ApiError::Other {
                    code: "invalid_url".into(),
                    message: format!("api URL scheme must be http or https, got `{other}`"),
                });
            }
        }
        Ok(Self {
            h2: build_h2()?,
            transport: OnceCell::new(),
            base: normalize_base(api),
            bearer: SecretString::from(String::new()),
            // Fall back to the default XDG path so a transport pin
            // written during `als auth login` survives to the next
            // command. A failure here just means "no persistence".
            config_path: als_core::config::config_path().ok(),
        })
    }

    /// Pick the transport client for this call. Resolution:
    ///
    /// 1. `ALS_TRANSPORT` env override wins outright.
    /// 2. `[transport."<host:port>"].protocol` from `config.toml`.
    /// 3. No entry → behave as `"auto"`.
    ///
    /// Force values (`"h3"`, `"h2"`) skip the probe. Anything else
    /// (`"auto"`, missing, unknown) probes HTTP/3 once and falls back
    /// to HTTP/2 on QUIC failure. The CLI never writes the result
    /// back — pinning is a user action.
    async fn http(&self) -> &HttpClient {
        self.transport
            .get_or_init(|| async { self.resolve_transport().await })
            .await
    }

    async fn resolve_transport(&self) -> HttpClient {
        let effective = env_transport().or_else(|| {
            host_port_key(&self.base)
                .zip(self.config_path.as_ref())
                .and_then(|(h, p)| als_core::config::read_transport(p, &h))
        });

        match effective.as_deref() {
            // Force h2: trust, no probe.
            Some(PROTOCOL_H2) => self.h2.clone(),
            // Force h3: trust, no probe, no fallback. A build failure
            // (e.g. quinn refusing because no tokio runtime, which
            // shouldn't happen here but is theoretically possible) is
            // the one fall-back path — surfacing it as a hard error
            // would be worse than silently using h2.
            Some(PROTOCOL_H3) => build_h3().unwrap_or_else(|| self.h2.clone()),
            // "auto", missing, unknown → probe and adapt in-memory.
            _ => match build_h3() {
                Some(h3) if probe_h3(&h3, &self.base).await => h3,
                _ => self.h2.clone(),
            },
        }
    }

    /// Compose a request URL against the configured base. Exposed as
    /// `pub(crate)` so endpoint modules and unit tests can assemble or
    /// inspect URLs without going through a full request.
    pub(crate) fn url(&self, path: &str) -> Result<Url, ApiError> {
        // Trim any accidental leading slash so the join always appends.
        let rel = path.trim_start_matches('/');
        self.base.join(rel).map_err(|e| ApiError::Other {
            code: "invalid_url".into(),
            message: e.to_string(),
        })
    }

    /// Render the `Authorization: Bearer …` header value. Kept private
    /// so the secret never escapes the client.
    fn bearer_value(&self) -> String {
        format!("Bearer {}", self.bearer.expose_secret())
    }

    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, ApiError> {
        let url = self.url(path)?;
        let req = self
            .http()
            .await
            .get(url)
            .header(reqwest::header::AUTHORIZATION, self.bearer_value())
            .query(query);
        self.send_json(req).await
    }

    pub(crate) async fn get_list<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<ListPayload<T>, ApiError> {
        let url = self.url(path)?;
        let req = self
            .http()
            .await
            .get(url)
            .header(reqwest::header::AUTHORIZATION, self.bearer_value())
            .query(query);
        self.send_list(req).await
    }

    pub(crate) async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = self.url(path)?;
        let req = self
            .http()
            .await
            .post(url)
            .header(reqwest::header::AUTHORIZATION, self.bearer_value())
            .json(body);
        self.send_json(req).await
    }

    pub(crate) async fn patch_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let url = self.url(path)?;
        let req = self
            .http()
            .await
            .patch(url)
            .header(reqwest::header::AUTHORIZATION, self.bearer_value())
            .json(body);
        self.send_json(req).await
    }

    pub(crate) async fn delete(&self, path: &str) -> Result<(), ApiError> {
        let url = self.url(path)?;
        let req = self
            .http()
            .await
            .delete(url)
            .header(reqwest::header::AUTHORIZATION, self.bearer_value());
        self.send_unit(req).await
    }

    pub(crate) async fn post_multipart<T: DeserializeOwned>(
        &self,
        path: &str,
        form: multipart::Form,
    ) -> Result<T, ApiError> {
        let url = self.url(path)?;
        let req = self
            .http()
            .await
            .post(url)
            .header(reqwest::header::AUTHORIZATION, self.bearer_value())
            .multipart(form);
        self.send_json(req).await
    }

    /// Unauthenticated `application/x-www-form-urlencoded` POST that
    /// returns the response body verbatim as a `(StatusCode, String)`
    /// pair. Used by the RFC 8628 device-flow endpoints, whose success
    /// and error bodies are flat JSON (no Toss envelope) — the caller
    /// is responsible for shape detection.
    pub(crate) async fn unauth_post_form_raw<B: Serialize>(
        &self,
        path: &str,
        form: &B,
    ) -> Result<(StatusCode, String), ApiError> {
        let url = self.url(path)?;
        let resp = self.http().await.post(url).form(form).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        Ok((status, body))
    }

    async fn send_json<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T, ApiError> {
        let resp = req.send().await?;
        let status = resp.status();
        let retry_after = parse_retry_after(&resp);
        let body = resp.text().await?;
        match parse_envelope::<T>(&body, status) {
            Err(ApiError::RateLimited(_)) => Err(ApiError::RateLimited(retry_after)),
            other => other,
        }
    }

    async fn send_list<T: DeserializeOwned>(
        &self,
        req: RequestBuilder,
    ) -> Result<ListPayload<T>, ApiError> {
        let resp = req.send().await?;
        let status = resp.status();
        let retry_after = parse_retry_after(&resp);
        let body = resp.text().await?;
        match parse_list_envelope::<T>(&body, status) {
            Err(ApiError::RateLimited(_)) => Err(ApiError::RateLimited(retry_after)),
            other => other,
        }
    }

    async fn send_unit(&self, req: RequestBuilder) -> Result<(), ApiError> {
        let resp = req.send().await?;
        let status = resp.status();
        let retry_after = parse_retry_after(&resp);
        if status == StatusCode::NO_CONTENT {
            return Ok(());
        }
        let body = resp.text().await?;
        if status.is_success() && body.trim().is_empty() {
            return Ok(());
        }
        match parse_envelope_no_data(&body, status) {
            Err(ApiError::RateLimited(_)) => Err(ApiError::RateLimited(retry_after)),
            other => other,
        }
    }
}

fn parse_retry_after(resp: &Response) -> u64 {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use std::path::PathBuf;

    use secrecy::SecretString;
    use url::Url;

    use super::*;

    fn test_config(api: &str, token: &str) -> als_core::Config {
        als_core::Config {
            api: Url::parse(api).unwrap(),
            token: SecretString::from(token.to_owned()),
            active_profile: None,
            source_path: PathBuf::from("/tmp/test.toml"),
        }
    }

    fn test_client(api: &str, token: &str) -> Client {
        Client::new(&test_config(api, token)).unwrap()
    }

    #[test]
    fn normalize_base_adds_api_for_bare_host() {
        let u = normalize_base(Url::parse("https://api.a.ls").unwrap());
        assert_eq!(u.as_str(), "https://api.a.ls/api/");
    }

    #[test]
    fn normalize_base_appends_slash_for_bare_api() {
        let u = normalize_base(Url::parse("https://api.a.ls/api").unwrap());
        assert_eq!(u.as_str(), "https://api.a.ls/api/");
    }

    #[test]
    fn normalize_base_preserves_full_api_slash() {
        let u = normalize_base(Url::parse("https://api.a.ls/api/").unwrap());
        assert_eq!(u.as_str(), "https://api.a.ls/api/");
    }

    #[test]
    fn url_composes_with_bare_host() {
        let c = test_client("https://api.a.ls", "tk_x");
        let u = c.url("sites").unwrap();
        assert_eq!(u.as_str(), "https://api.a.ls/api/sites");
    }

    #[test]
    fn url_composes_with_api_path() {
        let c = test_client("https://api.a.ls/api/", "tk_x");
        let u = c.url("sites").unwrap();
        assert_eq!(u.as_str(), "https://api.a.ls/api/sites");
    }

    #[test]
    fn url_strips_accidental_leading_slash() {
        let c = test_client("https://api.a.ls", "tk_x");
        let u = c.url("/sites").unwrap();
        assert_eq!(u.as_str(), "https://api.a.ls/api/sites");
    }

    #[test]
    fn url_preserves_path_segments_with_ids() {
        let c = test_client("https://api.a.ls", "tk_x");
        let u = c.url("sites/k7x2qm4j6p").unwrap();
        assert_eq!(u.as_str(), "https://api.a.ls/api/sites/k7x2qm4j6p");
    }

    #[test]
    fn bearer_value_uses_token() {
        let c = test_client("https://api.a.ls", "tk_secret_value");
        assert_eq!(c.bearer_value(), "Bearer tk_secret_value");
    }

    #[test]
    fn authenticated_request_carries_bearer_header() {
        let c = test_client("https://api.a.ls", "tk_abc123");
        let req =
            c.h2.get(c.url("account/me").unwrap())
                .header(reqwest::header::AUTHORIZATION, c.bearer_value())
                .build()
                .unwrap();
        let got = req
            .headers()
            .get(reqwest::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert_eq!(got, "Bearer tk_abc123");
        assert_eq!(req.url().path(), "/api/account/me");
    }

    #[test]
    fn unauth_request_has_no_authorization_header() {
        let c = test_client("https://api.a.ls", "tk_abc");
        let req =
            c.h2.post(c.url("auth/device/token").unwrap())
                .build()
                .unwrap();
        assert!(
            req.headers().get(reqwest::header::AUTHORIZATION).is_none(),
            "unauthenticated path must omit Authorization header"
        );
    }

    #[test]
    fn query_parameters_are_attached() {
        let c = test_client("https://api.a.ls", "tk_x");
        let req =
            c.h2.get(c.url("sites").unwrap())
                .header(reqwest::header::AUTHORIZATION, c.bearer_value())
                .query(&[("page", "1"), ("limit", "50")])
                .build()
                .unwrap();
        let q = req.url().query().unwrap_or("");
        assert!(q.contains("page=1"), "query = {q}");
        assert!(q.contains("limit=50"), "query = {q}");
    }

    #[test]
    fn client_new_builds_against_workspace_reqwest() {
        let _ = test_client("https://api.a.ls", "tk_x");
    }

    #[test]
    fn unauthenticated_builds_and_omits_bearer_header() {
        let api = Url::parse("https://api.a.ls").unwrap();
        let c = Client::unauthenticated(api).unwrap();
        let req =
            c.h2.post(c.url("auth/device/authorize").unwrap())
                .build()
                .unwrap();
        assert!(
            req.headers().get(reqwest::header::AUTHORIZATION).is_none(),
            "unauthenticated client must not pre-populate Authorization"
        );
    }

    #[test]
    fn host_port_key_collapses_path_variants() {
        let a = Url::parse("https://api.a.ls").unwrap();
        let b = Url::parse("https://api.a.ls/api/").unwrap();
        let c = Url::parse("https://api.a.ls/foo/bar").unwrap();
        assert_eq!(host_port_key(&a).as_deref(), Some("api.a.ls:443"));
        assert_eq!(host_port_key(&a), host_port_key(&b));
        assert_eq!(host_port_key(&a), host_port_key(&c));
    }

    #[test]
    fn host_port_key_respects_explicit_port() {
        let u = Url::parse("https://api.a.ls:8443/api").unwrap();
        assert_eq!(host_port_key(&u).as_deref(), Some("api.a.ls:8443"));
    }

    #[tokio::test]
    async fn http_transport_resolves_inside_runtime() {
        // The HTTP/3 builder uses `tokio::runtime::Handle::current()`
        // so resolution has to happen inside an async context. We
        // can't reach the real API base here, so the probe will fail
        // and the OnceCell ends up holding a clone of `h2` — but the
        // important property is just that `http()` returns and the
        // cell stays populated for subsequent calls.
        let c = test_client("https://127.0.0.1:65535", "tk_x");
        let first: *const HttpClient = c.http().await;
        let second: *const HttpClient = c.http().await;
        assert_eq!(
            first, second,
            "OnceCell must cache the resolved transport for the rest of the process"
        );
    }

    #[test]
    fn unauthenticated_rejects_non_http_scheme() {
        // A `file://` URL would otherwise let an `auth login` against
        // a hand-written `config.toml` walk into the local filesystem
        // before reqwest reports the unsupported-scheme failure.
        let api = Url::parse("file:///tmp/whatever").unwrap();
        match Client::unauthenticated(api) {
            Ok(_) => panic!("file:// scheme must be rejected"),
            Err(ApiError::Other { code, message }) => {
                assert_eq!(code, "invalid_url");
                assert!(message.contains("file"), "msg = {message}");
            }
            Err(other) => panic!("unexpected variant: {other:?}"),
        }
    }
}
