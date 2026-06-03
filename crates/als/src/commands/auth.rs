//! `als auth …` — user-identity commands.
//!
//! Subcommands:
//!
//! * `login`  — pair this device via the RFC 8628 device-flow, or store
//!   a pre-minted token with `--token`.
//! * `logout` — clear the local token (does **not** revoke server-side).
//! * `revoke` — revoke the local token server-side, then clear locally.
//! * `status` — show the active profile's user + quota footer.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

use als_api::endpoints::{account, device, quota, tokens};
use als_api::{AccountMe, Client, DeviceTokenResponse, Quota};
use als_core::config::{ConfigFile, Profile, config_path, save};
use als_core::{Error, Output, OutputMode, config};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use crate::cli::{AuthSub, LoginArgs, RevokeArgs};
use crate::commands::qr;
use crate::prompt;

/// Hard ceiling for the device-flow polling interval, per
/// `cli-server-contract.md` in the als-docs repo (the server also
/// bumps its own copy).
const POLL_INTERVAL_CAP_SECS: u64 = 30;
/// Env override used by the integration tests to collapse the poll
/// sleep so wall-clock stays under a couple of seconds. Production
/// callers leave it unset and the server-provided `interval` is used
/// verbatim.
const POLL_OVERRIDE_ENV: &str = "ALS_LOGIN_POLL_MS";

pub(crate) async fn run(
    out: &mut Output,
    profile: Option<&str>,
    sub: AuthSub,
) -> Result<(), Error> {
    match sub {
        AuthSub::Login(args) => login(out, profile, args).await,
        AuthSub::Logout(_) => logout(out, profile),
        AuthSub::Revoke(args) => revoke(out, profile, args).await,
        AuthSub::Status => status(out, profile).await,
    }
}

async fn login(out: &mut Output, profile: Option<&str>, args: LoginArgs) -> Result<(), Error> {
    if let Some(token) = args.token.as_deref() {
        // CI / scripted handoffs paste tokens directly via `--token`. A
        // typo or accidental shell mangling would otherwise persist to
        // `config.toml` and surface as an opaque 401 on the next API
        // call. Reject obviously-broken shapes (empty, whitespace,
        // non-printable) up front so the failure stays near the user
        // who typed it.
        validate_token_shape(token)?;
        return persist_token(out, profile, token);
    }
    let api = api_url_from_config(profile)?;
    let client = Client::unauthenticated(api).map_err(Error::from)?;
    let hint = hostname_hint();
    let auth = device::authorize(&client, hint.as_deref()).await?;

    render_login_card(out, &auth, args.no_qr);

    let token = poll_until_authorized(&client, &auth.device_code, auth.interval, auth.expires_in)
        .await
        .map_err(|kind| {
            emit_terminal_login_error(out, kind);
            Error::Other(kind.sentinel().to_owned())
        })?;

    persist_token(out, profile, &token)
}

/// Render the user-code, verification URL, and optional QR to stdout
/// in Human / Quiet mode. JSON mode prints the JSON envelope to stdout
/// once at the end (matching `als auth status`); during login we just
/// suppress decoration.
fn render_login_card(out: &mut Output, auth: &als_api::DeviceAuthorization, no_qr: bool) {
    let want_qr = !no_qr && std::io::stdout().is_terminal();
    let user_code = &auth.user_code;
    let complete = &auth.verification_uri_complete;
    let plain = &auth.verification_uri;
    out.human(|w| {
        let _ = writeln!(w, "\nTo authorize this device, visit:");
        let _ = writeln!(w, "  \x1b[1m{plain}\x1b[0m");
        let _ = writeln!(w, "and enter the code:");
        let _ = writeln!(w, "  \x1b[1;36m{user_code}\x1b[0m");
        let _ = writeln!(w, "Or open this one-tap link in any signed-in browser:");
        let _ = writeln!(w, "  {complete}");
        if want_qr {
            let _ = writeln!(w);
            let _ = qr::render(w, complete);
        }
        let _ = writeln!(w, "\nWaiting for authorization...");
    });
    if out.mode() == OutputMode::Quiet {
        out.quiet_line(&format!("verification_uri: {plain}"));
        out.quiet_line(&format!("user_code: {user_code}"));
        out.quiet_line(&format!("verification_uri_complete: {complete}"));
    }
}

/// Sentinel-bearing failure modes for the polling state machine. Each
/// maps to one of the `Error::Other(...)` strings the binary's
/// `report_error` already special-cases.
#[derive(Debug, Clone, Copy)]
enum LoginFailure {
    Denied,
    Timeout,
}

impl LoginFailure {
    fn sentinel(self) -> &'static str {
        match self {
            LoginFailure::Denied => "login_denied",
            LoginFailure::Timeout => "login_timeout",
        }
    }
}

fn emit_terminal_login_error(out: &mut Output, kind: LoginFailure) {
    match kind {
        LoginFailure::Denied => out.err("login_denied", "device authorization was denied", None),
        LoginFailure::Timeout => out.err("login_timeout", "device code expired", None),
    }
}

async fn poll_until_authorized(
    client: &Client,
    device_code: &str,
    interval_secs: u64,
    expires_in_secs: u64,
) -> Result<String, LoginFailure> {
    let mut interval = interval_secs.max(1);
    let override_ms = env_u64(POLL_OVERRIDE_ENV);
    let deadline = Instant::now() + Duration::from_secs(expires_in_secs);
    loop {
        sleep(interval, override_ms).await;
        if Instant::now() >= deadline {
            return Err(LoginFailure::Timeout);
        }
        match device::poll(client, device_code).await {
            Ok(DeviceTokenResponse::Granted { access_token, .. }) => return Ok(access_token),
            Ok(DeviceTokenResponse::Pending) => {}
            Ok(DeviceTokenResponse::SlowDown) => {
                interval = next_interval_after_slow_down(interval);
            }
            Ok(DeviceTokenResponse::Denied) => return Err(LoginFailure::Denied),
            // Transport / decode errors collapse into the same timeout
            // sentinel as `expired_token` — both end the flow with
            // exit 1 and tell the operator to re-run `als login`.
            Ok(DeviceTokenResponse::Expired) | Err(_) => return Err(LoginFailure::Timeout),
        }
    }
}

/// Double the polling interval after a `slow_down` reply, capped at
/// [`POLL_INTERVAL_CAP_SECS`] per `cli-server-contract.md` in the
/// als-docs repo. Kept as a pure function so the doubling rule has
/// direct unit coverage.
fn next_interval_after_slow_down(current: u64) -> u64 {
    current.saturating_mul(2).min(POLL_INTERVAL_CAP_SECS)
}

async fn sleep(interval_secs: u64, override_ms: Option<u64>) {
    let dur = match override_ms {
        Some(ms) => Duration::from_millis(ms),
        None => Duration::from_secs(interval_secs),
    };
    tokio::time::sleep(dur).await;
}

fn hostname_hint() -> Option<String> {
    if let Ok(env) = std::env::var("HOSTNAME")
        && !env.is_empty()
    {
        return Some(env);
    }
    hostname::get()
        .ok()
        .and_then(|os| os.into_string().ok())
        .filter(|s| !s.is_empty())
}

fn persist_token(out: &mut Output, profile: Option<&str>, token: &str) -> Result<(), Error> {
    write_token(profile.unwrap_or("default"), Some(token))?;
    out.quiet_line("\x1b[32m✓\x1b[0m Logged in.");
    Ok(())
}

fn logout(out: &mut Output, profile: Option<&str>) -> Result<(), Error> {
    write_token(profile.unwrap_or("default"), None)?;
    out.quiet_line("\x1b[32m✓\x1b[0m Local token cleared.");
    Ok(())
}

fn write_token(profile_name: &str, token: Option<&str>) -> Result<(), Error> {
    let path = config_path()?;
    let mut file = load_or_default(&path)?;
    let target: &mut Profile = if profile_name == "default" {
        file.default.get_or_insert_with(Profile::default)
    } else {
        file.profile.entry(profile_name.to_owned()).or_default()
    };
    target.token = token.map(str::to_owned).filter(|s| !s.is_empty());
    save(&file, &path)
}

fn load_or_default(path: &std::path::Path) -> Result<ConfigFile, Error> {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s)
            .map_err(|e| Error::Config(format!("failed to parse {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile::default()),
        Err(e) => Err(Error::Io(e)),
    }
}

fn api_url_from_config(profile: Option<&str>) -> Result<url::Url, Error> {
    if let Ok(cfg) = config::load(profile) {
        return Ok(cfg.api);
    }
    // No token yet (typical for first `auth login`); read the raw config
    // file's `api` field if present, else fall back to the built-in
    // default so the CLI works out of the box.
    let path = config_path()?;
    let file = load_or_default(&path)?;
    let profile_name = profile.unwrap_or("default");
    let raw = if profile_name == "default" {
        file.default.as_ref().and_then(|p| p.api.clone())
    } else {
        file.profile.get(profile_name).and_then(|p| p.api.clone())
    }
    .unwrap_or_else(|| als_core::DEFAULT_API_URL.to_owned());
    url::Url::parse(&raw).map_err(|e| Error::Config(format!("invalid api URL: {e}")))
}

async fn revoke(out: &mut Output, profile: Option<&str>, args: RevokeArgs) -> Result<(), Error> {
    let cfg = config::load(profile)?;
    // Only the first 8 characters of the token leave the `SecretString`;
    // the full value never lands in a plain `String` that would skip the
    // zeroize-on-drop guarantee.
    let prefix = token_prefix(&cfg.token);
    if prefix.is_empty() {
        return Err(Error::Auth);
    }
    if !args.yes
        && !prompt::confirm(&format!(
            "Revoke server-side token '{prefix}…' and clear locally?"
        ))?
    {
        out.quiet_line("Cancelled.");
        return Ok(());
    }
    let client = Client::new(&cfg).map_err(Error::from)?;
    tokens::delete(&client, &prefix).await?;
    write_token(profile.unwrap_or("default"), None)?;
    out.quiet_line("\x1b[32m✓\x1b[0m Token revoked server-side and cleared locally.");
    Ok(())
}

#[derive(Serialize)]
struct StatusJson<'a> {
    user: &'a AccountMe,
    quota: &'a Quota,
    token_prefix: String,
    api: String,
    profile: String,
}

async fn status(out: &mut Output, profile: Option<&str>) -> Result<(), Error> {
    let cfg = config::load(profile)?;
    let client = Client::new(&cfg).map_err(Error::from)?;
    let (user_res, quota_res) = tokio::join!(account::me(&client), quota::me(&client));
    let user = user_res?;
    let q = quota_res?;
    let prefix = token_prefix(&cfg.token);
    match out.mode() {
        OutputMode::Json => out.json(&StatusJson {
            user: &user,
            quota: &q,
            token_prefix: prefix.clone(),
            api: cfg.api.to_string(),
            profile: cfg
                .active_profile
                .clone()
                .unwrap_or_else(|| "default".into()),
        })?,
        _ => out.human(|w| write_status_human(w, &user, &q, &prefix, &cfg)),
    }
    Ok(())
}

/// Hard ceiling for `--token` lengths. The server tokens are ~32–64
/// characters in practice; cap at 256 so a stray paste of a multi-line
/// blob fails fast instead of bloating `config.toml`.
const MAX_TOKEN_LEN: usize = 256;
/// Minimum `--token` length. The shortest legitimate token would still
/// comfortably clear this bound; the goal is to reject obvious typos
/// like a one-character paste.
const MIN_TOKEN_LEN: usize = 8;

/// Reject obviously-malformed `--token` values before they reach disk.
/// Server-side validation is still authoritative; this is a fast-path
/// guard so a CI operator's typo surfaces immediately.
fn validate_token_shape(token: &str) -> Result<(), Error> {
    if token.is_empty() {
        return Err(Error::Config("--token value cannot be empty".into()));
    }
    if token.len() < MIN_TOKEN_LEN || token.len() > MAX_TOKEN_LEN {
        return Err(Error::Config(format!(
            "--token length {} is outside the expected {MIN_TOKEN_LEN}..={MAX_TOKEN_LEN} character range",
            token.len()
        )));
    }
    if !token.chars().all(|c| c.is_ascii_graphic()) {
        return Err(Error::Config(
            "--token must contain only printable ASCII (no spaces, tabs, or non-ASCII characters)"
                .into(),
        ));
    }
    Ok(())
}

fn token_prefix(secret: &SecretString) -> String {
    let raw = secret.expose_secret();
    if raw.is_empty() {
        return String::new();
    }
    raw.chars().take(8).collect()
}

fn write_status_human(
    w: &mut dyn Write,
    user: &AccountMe,
    q: &Quota,
    token_prefix: &str,
    cfg: &als_core::Config,
) {
    let profile = cfg.active_profile.as_deref().unwrap_or("default");
    let _ = writeln!(w, "Profile: {profile}");
    let _ = writeln!(w, "User:    {} <{}>", user.name, user.email);
    let _ = writeln!(w, "API:     {}", cfg.api);
    if token_prefix.is_empty() {
        let _ = writeln!(w, "Token:   (not set)");
    } else {
        let _ = writeln!(w, "Token:   {token_prefix}** ({}…)", &token_prefix);
    }
    let cap = q
        .site_cap()
        .map_or_else(|| "∞".to_owned(), |n| n.to_string());
    let _ = writeln!(
        w,
        "Quota:   {} / {} sites · {} / {} per-archive",
        q.total_sites,
        cap,
        format_bytes(q.total_bytes),
        format_bytes(q.max_size_bytes)
    );
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    #[allow(clippy::cast_precision_loss)]
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|s| s.parse::<u64>().ok())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    #[test]
    fn token_prefix_returns_first_eight() {
        let s = SecretString::from("tk_abcdef1234567".to_string());
        assert_eq!(token_prefix(&s), "tk_abcde");
    }

    #[test]
    fn token_prefix_handles_empty() {
        let s = SecretString::from(String::new());
        assert_eq!(token_prefix(&s), "");
    }

    #[test]
    fn format_bytes_basic() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn login_failure_sentinels_match_report_error() {
        assert_eq!(LoginFailure::Denied.sentinel(), "login_denied");
        assert_eq!(LoginFailure::Timeout.sentinel(), "login_timeout");
    }

    #[test]
    fn slow_down_doubles_interval() {
        assert_eq!(next_interval_after_slow_down(5), 10);
        assert_eq!(next_interval_after_slow_down(10), 20);
    }

    #[test]
    fn validate_token_shape_accepts_typical_token() {
        validate_token_shape("tk_abcdef1234567890").expect("typical token");
    }

    #[test]
    fn validate_token_shape_rejects_empty_and_short() {
        assert!(validate_token_shape("").is_err());
        assert!(validate_token_shape("short").is_err());
    }

    #[test]
    fn validate_token_shape_rejects_whitespace_and_non_ascii() {
        assert!(validate_token_shape("tk_abc def123").is_err());
        assert!(validate_token_shape("tk_abc\tdef123").is_err());
        assert!(validate_token_shape("tk_abc\ndef1234").is_err());
        assert!(validate_token_shape("tk_unicode_😀_padding").is_err());
    }

    #[test]
    fn validate_token_shape_rejects_oversize() {
        let oversize = "x".repeat(MAX_TOKEN_LEN + 1);
        assert!(validate_token_shape(&oversize).is_err());
    }

    #[test]
    fn slow_down_caps_interval_at_thirty_seconds() {
        assert_eq!(next_interval_after_slow_down(20), POLL_INTERVAL_CAP_SECS);
        assert_eq!(
            next_interval_after_slow_down(POLL_INTERVAL_CAP_SECS),
            POLL_INTERVAL_CAP_SECS
        );
        // Saturation guard.
        assert_eq!(
            next_interval_after_slow_down(u64::MAX),
            POLL_INTERVAL_CAP_SECS
        );
    }
}
