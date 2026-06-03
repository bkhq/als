//! `sites/:id/password` endpoint.
//!
//! One POST endpoint serves both "set / replace" and "clear":
//!
//! * `password: "auto"` — server generates and returns a memorable
//!   password in the response.
//! * `password: "<plaintext>"` — server stores it verbatim.
//! * `password: null` — clears the password (makes the site public).

use serde::{Deserialize, Serialize};

use crate::Client;
use crate::endpoints::sites::Site;
use crate::error::ApiError;

#[derive(Debug, Serialize)]
struct WirePasswordBody<'a> {
    /// `Some` → set / replace; `None` (serialised as `null`) → clear.
    password: Option<&'a str>,
}

/// Server response for `POST sites/:id/password`. Carries the updated
/// site row plus an `op` discriminator (`"set"` / `"cleared"` /
/// `"rotated"` depending on the server's audit branch).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPasswordResponse {
    #[serde(flatten)]
    pub site: Site,
    /// Server-side audit hint: `"set"`, `"cleared"`, or `"rotated"`.
    #[serde(default)]
    pub op: Option<String>,
}

/// `POST sites/:id/password` — set, replace, or clear the site password.
///
/// * `Some("auto")` → the server generates a password and returns it via
///   the new site state. The server does not echo the plaintext back; if
///   you need to display the password, generate it client-side and send
///   the literal value instead.
/// * `Some(literal)` → store as the new password.
/// * `None` → clear (returns 422 `VALIDATION_ERROR` if the site has no
///   password to clear; the binary handles that case before calling).
pub async fn set(
    client: &Client,
    id: &str,
    password: Option<&str>,
) -> Result<SetPasswordResponse, ApiError> {
    let path = format!("sites/{id}/password");
    let body = WirePasswordBody { password };
    client.post_json(&path, &body).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_serializes_password() {
        let s = serde_json::to_string(&WirePasswordBody {
            password: Some("s3cret"),
        })
        .unwrap();
        assert_eq!(s, r#"{"password":"s3cret"}"#);
    }

    #[test]
    fn body_serializes_null_for_clear() {
        let s = serde_json::to_string(&WirePasswordBody { password: None }).unwrap();
        assert_eq!(s, r#"{"password":null}"#);
    }

    #[test]
    fn body_serializes_auto_sentinel() {
        let s = serde_json::to_string(&WirePasswordBody {
            password: Some("auto"),
        })
        .unwrap();
        assert_eq!(s, r#"{"password":"auto"}"#);
    }

    #[test]
    fn response_deserializes_with_op() {
        let body = r#"{
            "id": "k7x2qm4j6p",
            "fullDomain": "k7x2qm4j6p.a.ls",
            "projectName": "demo",
            "description": null,
            "authRequired": true,
            "currentVersion": "v1",
            "createdAt": "2026-05-09T10:30:22.000Z",
            "updatedAt": "2026-05-09T10:30:22.000Z",
            "expiresAt": null,
            "releasedAt": null,
            "op": "set"
        }"#;
        let resp: SetPasswordResponse = serde_json::from_str(body).unwrap();
        assert_eq!(resp.site.id, "k7x2qm4j6p");
        assert!(resp.site.auth_required);
        assert_eq!(resp.op.as_deref(), Some("set"));
    }
}
