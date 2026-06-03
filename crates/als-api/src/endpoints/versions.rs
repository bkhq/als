//! `sites/:id/versions` and `sites/:id/activate` endpoints.
//!
//! `list` returns the version history newest-first. `activate` switches
//! the live version. The live server does not currently implement
//! `activate`; the mock server does, so the binary keeps the call path
//! and reports the server's response verbatim (typically `404 NOT_FOUND`
//! against the real server).

use serde::{Deserialize, Serialize};

use crate::Client;
use crate::error::ApiError;

/// One row from `GET sites/:id/versions`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: String,
    pub site_id: String,
    /// ISO-8601 timestamp.
    pub created_at: String,
    pub size_bytes: u64,
    pub file_count: u32,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
struct ActivateBody<'a> {
    version: &'a str,
}

/// `GET sites/:id/versions` — list every retained version for the site
/// in server-defined order (newest first).
pub async fn list(client: &Client, id: &str) -> Result<Vec<Version>, ApiError> {
    let path = format!("sites/{id}/versions");
    client.get_json::<Vec<Version>>(&path, &[]).await
}

/// `POST sites/:id/activate` — point the live route at `version`.
///
/// The response body is ignored; callers that need the new
/// `currentVersion` should fetch the site after activate succeeds.
pub async fn activate(client: &Client, id: &str, version: &str) -> Result<(), ApiError> {
    let path = format!("sites/{id}/activate");
    let _: serde::de::IgnoredAny = client.post_json(&path, &ActivateBody { version }).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activate_body_serializes_to_single_field_object() {
        let body = ActivateBody {
            version: "20260510-103022-a3f1c2",
        };
        assert_eq!(
            serde_json::to_value(&body).expect("serialize"),
            serde_json::json!({ "version": "20260510-103022-a3f1c2" })
        );
    }

    #[test]
    fn version_deserializes_camelcase() {
        let body = r#"{
            "id": "20260510-103022-a3f1c2",
            "siteId": "k7x2qm4j6p",
            "createdAt": "2026-05-10T10:30:22.000Z",
            "sizeBytes": 1048576,
            "fileCount": 47,
            "note": "homepage refresh"
        }"#;
        let v: Version = serde_json::from_str(body).unwrap();
        assert_eq!(v.id, "20260510-103022-a3f1c2");
        assert_eq!(v.site_id, "k7x2qm4j6p");
        assert_eq!(v.size_bytes, 1_048_576);
        assert_eq!(v.file_count, 47);
        assert_eq!(v.note.as_deref(), Some("homepage refresh"));
    }
}
