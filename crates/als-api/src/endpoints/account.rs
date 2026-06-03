//! `GET account/me` — current user record.
//!
//! Consumed by `als auth status` and by `als auth login` after the
//! pairing flow to display the email associated with the newly saved
//! token. The live server returns a richer payload than the CLI uses
//! (`groups`, TOTP state, etc.); only the bits the CLI renders are
//! modelled here, leaning on serde's permissive "ignore unknown
//! fields" default.

use serde::{Deserialize, Serialize};

use crate::Client;
use crate::error::ApiError;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountMe {
    pub id: String,
    pub username: String,
    pub name: String,
    pub email: String,
    pub role: String,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// ISO-8601 timestamp.
    #[serde(default)]
    pub last_login_at: Option<String>,
    /// ISO-8601 timestamp.
    #[serde(default)]
    pub created_at: Option<String>,
}

pub async fn me(client: &Client) -> Result<AccountMe, ApiError> {
    client.get_json("account/me", &[]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_me_deserializes_typical_payload() {
        let body = r#"{
            "id": "u_alice",
            "username": "alice",
            "name": "Alice",
            "email": "alice@example.com",
            "avatar": null,
            "role": "user",
            "status": "active",
            "lastLoginAt": "2026-05-12T03:00:00.000Z",
            "createdAt": "2026-01-01T00:00:00.000Z",
            "groups": []
        }"#;
        let me: AccountMe = serde_json::from_str(body).unwrap();
        assert_eq!(me.id, "u_alice");
        assert_eq!(me.email, "alice@example.com");
        assert_eq!(me.role, "user");
    }

    #[test]
    fn account_me_tolerates_missing_optionals() {
        let body = r#"{
            "id": "u_alice",
            "username": "alice",
            "name": "Alice",
            "email": "alice@example.com",
            "role": "user"
        }"#;
        let me: AccountMe = serde_json::from_str(body).unwrap();
        assert_eq!(me.id, "u_alice");
        assert!(me.last_login_at.is_none());
    }
}
