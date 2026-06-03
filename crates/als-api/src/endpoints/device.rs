//! `/api/auth/device/*` endpoints — RFC 8628 device authorization grant.
//!
//! Two CLI-facing calls:
//!
//! * [`authorize`] — `POST /api/auth/device/authorize`. The CLI POSTs an
//!   `application/x-www-form-urlencoded` body (`client_id=toss-cli`,
//!   optional `hostname_hint`) and gets back the device + user codes,
//!   the two verification URIs, plus polling parameters.
//! * [`poll`] — `POST /api/auth/device/token`. The CLI POSTs the
//!   device-code grant and receives either the freshly minted bearer
//!   token (200) or one of the five RFC 8628 error codes (4xx).
//!
//! Both endpoints return RFC 8628 plain JSON shapes — **not** the Toss
//! `{success, data, error}` envelope. The transport helper used here
//! (`Client::unauth_post_form_raw`) returns the body verbatim so this
//! module can parse the flat shape.

use serde::{Deserialize, Serialize};

use crate::Client;
use crate::error::ApiError;

const CLIENT_ID: &str = "toss-cli";
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Response of `POST /api/auth/device/authorize` (RFC 8628 §3.2).
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Result of one `POST /api/auth/device/token` poll. `Granted` carries
/// the freshly minted bearer token; everything else maps onto a
/// RFC 8628 §3.5 error code.
#[derive(Debug, Clone)]
pub enum DeviceTokenResponse {
    Granted {
        access_token: String,
        expires_in: u64,
    },
    Pending,
    SlowDown,
    Denied,
    Expired,
}

#[derive(Serialize)]
struct AuthorizeForm<'a> {
    client_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    hostname_hint: Option<&'a str>,
}

#[derive(Serialize)]
struct TokenForm<'a> {
    grant_type: &'a str,
    device_code: &'a str,
    client_id: &'a str,
}

/// Plain success shape returned by `/token` on 200.
#[derive(Deserialize)]
struct TokenSuccess {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

/// Plain error shape returned by `/token` on 4xx.
#[derive(Deserialize)]
struct TokenError {
    error: String,
}

/// `POST /api/auth/device/authorize` — start a device-flow grant.
///
/// `hostname_hint` is optional and surfaces in the browser confirmation
/// card so the user can recognise which machine is asking. Pass the
/// machine's hostname if available; the server falls back to a generic
/// label when absent.
pub async fn authorize(
    client: &Client,
    hostname_hint: Option<&str>,
) -> Result<DeviceAuthorization, ApiError> {
    let form = AuthorizeForm {
        client_id: CLIENT_ID,
        hostname_hint,
    };
    let (status, body) = client
        .unauth_post_form_raw("auth/device/authorize", &form)
        .await?;
    if !status.is_success() {
        return Err(plain_error_to_api(&body, status));
    }
    serde_json::from_str::<DeviceAuthorization>(&body)
        .map_err(|e| ApiError::Decode(format!("device authorize: {e}")))
}

/// `POST /api/auth/device/token` — one poll.
///
/// Returns:
/// * `Granted` — server minted a token; persist it.
/// * `Pending` — user has not approved yet; keep polling at the current
///   interval.
/// * `SlowDown` — caller is polling faster than the server's minimum
///   gap; double the interval (cap 30 s).
/// * `Denied` — user clicked `Deny` in the browser.
/// * `Expired` — `device_code` past its TTL or already claimed.
///
/// Any other RFC 8628 error code (`invalid_client`,
/// `unsupported_grant_type`, …) surfaces as [`ApiError::Other`]; these
/// indicate a client bug and should not normally be encountered.
pub async fn poll(client: &Client, device_code: &str) -> Result<DeviceTokenResponse, ApiError> {
    let form = TokenForm {
        grant_type: GRANT_TYPE,
        device_code,
        client_id: CLIENT_ID,
    };
    let (status, body) = client
        .unauth_post_form_raw("auth/device/token", &form)
        .await?;
    if status.is_success() {
        let ok = serde_json::from_str::<TokenSuccess>(&body)
            .map_err(|e| ApiError::Decode(format!("device token: {e}")))?;
        return Ok(DeviceTokenResponse::Granted {
            access_token: ok.access_token,
            expires_in: ok.expires_in,
        });
    }
    let err = serde_json::from_str::<TokenError>(&body)
        .map_err(|e| ApiError::Decode(format!("device token error: {e}")))?;
    Ok(match err.error.as_str() {
        "authorization_pending" => DeviceTokenResponse::Pending,
        "slow_down" => DeviceTokenResponse::SlowDown,
        "access_denied" => DeviceTokenResponse::Denied,
        "expired_token" => DeviceTokenResponse::Expired,
        other => {
            return Err(ApiError::Other {
                code: other.to_owned(),
                message: format!("RFC 8628 error: {other}"),
            });
        }
    })
}

/// Map an RFC 8628 plain `{error: ...}` body (with non-2xx status) to
/// `ApiError`. Used by `authorize`, which has no token-poll branches —
/// every non-2xx is just a request-level failure.
fn plain_error_to_api(body: &str, status: reqwest::StatusCode) -> ApiError {
    match serde_json::from_str::<TokenError>(body) {
        Ok(err) => ApiError::Other {
            code: err.error,
            message: format!("device authorize failed (HTTP {})", status.as_u16()),
        },
        Err(_) if status == reqwest::StatusCode::TOO_MANY_REQUESTS => ApiError::RateLimited(0),
        Err(_) => ApiError::Decode(format!(
            "device authorize: unexpected HTTP {} body: {body}",
            status.as_u16()
        )),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn device_authorization_deserializes_rfc_shape() {
        let body = r#"{
            "device_code": "dc_abc",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://a.ls/verify",
            "verification_uri_complete": "https://a.ls/verify?user_code=ABCD-EFGH",
            "expires_in": 900,
            "interval": 5
        }"#;
        let r: DeviceAuthorization = serde_json::from_str(body).unwrap();
        assert_eq!(r.user_code, "ABCD-EFGH");
        assert_eq!(r.expires_in, 900);
        assert_eq!(r.interval, 5);
    }

    #[test]
    fn token_success_deserializes() {
        let body = r#"{"access_token":"tk_abc","token_type":"Bearer","expires_in":7200}"#;
        let r: TokenSuccess = serde_json::from_str(body).unwrap();
        assert_eq!(r.access_token, "tk_abc");
        assert_eq!(r.expires_in, 7200);
    }

    #[test]
    fn token_error_deserializes_each_rfc_code() {
        for code in [
            "authorization_pending",
            "slow_down",
            "access_denied",
            "expired_token",
        ] {
            let body = format!(r#"{{"error":"{code}"}}"#);
            let r: TokenError = serde_json::from_str(&body).unwrap();
            assert_eq!(r.error, code);
        }
    }
}
