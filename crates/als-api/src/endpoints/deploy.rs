//! `POST /api/deploy` — upload a zipped site as a multipart request.
//!
//! Form fields (only `archive` is required):
//!
//! | name           | value                                                     |
//! |----------------|-----------------------------------------------------------|
//! | `archive`      | binary `application/zip`                                  |
//! | `project_name` | string                                                    |
//! | `expires`      | duration string (`24h`, `7d`, `never`, RFC 3339 timestamp)|
//! | `auth_password`| 8–200 char string                                         |
//!
//! The server response carries the resulting site id, public URL,
//! project name, version id, ISO expiry, and the boolean
//! `authRequired`. The binary derives any extra metadata it needs
//! (archive size, file count, …) from the local archive.

use reqwest::multipart;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::error::ApiError;

const ARCHIVE_MIME: &str = "application/zip";

/// Optional metadata that accompanies the uploaded archive.
///
/// Field semantics, server-side:
/// * `site_id` is the authoritative identity key. When present and the
///   caller owns it, the server pushes a new version to that site and
///   ignores `project_name` for identification (it keeps the
///   site's existing name; the response surfaces whatever the server
///   actually has so the CLI can refresh a stale local pin). Unknown
///   or non-owned id → `404 site_not_found`.
/// * `project_name` is the fallback identity key. When `site_id` is
///   absent, the server applies its case-insensitive per-user
///   name-uniqueness check: a hit pushes a new version, a miss
///   allocates a fresh site under that name.
/// * `expires` / `auth_password` are passed through verbatim and only
///   override existing state when supplied.
///
/// `expires` is rendered via [`als_core::Expires`]'s `Display` impl,
/// which produces the canonical wire form (`24h`, `never`, RFC 3339, …).
#[derive(Debug, Default, Clone)]
pub struct DeployRequest {
    pub site_id: Option<String>,
    pub project_name: Option<String>,
    pub expires: Option<als_core::Expires>,
    pub auth_password: Option<String>,
}

/// Deserialized `data` payload of a successful `POST /api/deploy`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployResponse {
    pub id: String,
    pub url: String,
    pub project_name: String,
    pub version: String,
    /// ISO-8601 timestamp, or `null` for "never expires".
    pub expires_at: Option<String>,
    pub auth_required: bool,
}

/// Text parts emitted by [`DeployRequest`] when present.
///
/// Returns owned `(name, value)` pairs in a stable order so unit tests
/// can assert exact membership without depending on multipart-stream
/// framing.
fn text_parts(request: &DeployRequest) -> Vec<(&'static str, String)> {
    let mut parts = Vec::with_capacity(4);
    if let Some(v) = request.site_id.as_ref() {
        parts.push(("site_id", v.clone()));
    }
    if let Some(v) = request.project_name.as_ref() {
        parts.push(("project_name", v.clone()));
    }
    if let Some(v) = request.expires.as_ref() {
        parts.push(("expires", v.to_string()));
    }
    if let Some(v) = request.auth_password.as_ref() {
        parts.push(("auth_password", v.clone()));
    }
    parts
}

/// Build the multipart form for `POST /api/deploy`.
///
/// Kept as a free helper so the caller (and unit tests) can construct
/// the form without going through the network layer.
fn build_form(
    archive_bytes: Vec<u8>,
    archive_filename: String,
    request: &DeployRequest,
) -> Result<multipart::Form, ApiError> {
    let archive_part = multipart::Part::bytes(archive_bytes)
        .file_name(archive_filename)
        .mime_str(ARCHIVE_MIME)?;
    let mut form = multipart::Form::new().part("archive", archive_part);
    for (name, value) in text_parts(request) {
        form = form.text(name, value);
    }
    Ok(form)
}

/// `POST /api/deploy` — upload a zipped site and return the created site
/// descriptor.
pub async fn deploy(
    client: &Client,
    archive_bytes: Vec<u8>,
    archive_filename: String,
    request: DeployRequest,
) -> Result<DeployResponse, ApiError> {
    let form = build_form(archive_bytes, archive_filename, &request)?;
    client.post_multipart("deploy", form).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::duration_suboptimal_units)]

    use std::time::Duration;

    use super::*;

    fn req_with_all_fields() -> DeployRequest {
        DeployRequest {
            site_id: Some("k7x2qm4j6p".into()),
            project_name: Some("hello".into()),
            expires: Some(als_core::Expires::Duration(Duration::from_secs(24 * 3_600))),
            auth_password: Some("s3cret".into()),
        }
    }

    #[test]
    fn text_parts_emits_nothing_when_empty() {
        let parts = text_parts(&DeployRequest::default());
        assert!(parts.is_empty(), "parts = {parts:?}");
    }

    #[test]
    fn text_parts_emits_only_present_fields() {
        let req = DeployRequest {
            project_name: Some("hello".into()),
            ..DeployRequest::default()
        };
        let parts = text_parts(&req);
        assert_eq!(parts, vec![("project_name", "hello".into())]);
    }

    #[test]
    fn text_parts_renders_expires_via_display() {
        let req = DeployRequest {
            expires: Some(als_core::Expires::Never),
            ..DeployRequest::default()
        };
        let parts = text_parts(&req);
        assert_eq!(parts, vec![("expires", "never".into())]);
    }

    #[test]
    fn text_parts_renders_duration_expires_in_hours() {
        let req = DeployRequest {
            expires: Some(als_core::Expires::Duration(Duration::from_secs(
                168 * 3_600,
            ))),
            ..DeployRequest::default()
        };
        let parts = text_parts(&req);
        assert_eq!(parts, vec![("expires", "168h".into())]);
    }

    #[test]
    fn text_parts_preserves_documented_field_order() {
        let parts = text_parts(&req_with_all_fields());
        let names: Vec<&'static str> = parts.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec!["site_id", "project_name", "expires", "auth_password",]
        );
    }

    #[test]
    fn text_parts_emits_only_site_id_when_isolated() {
        let req = DeployRequest {
            site_id: Some("k7x2qm4j6p".into()),
            ..DeployRequest::default()
        };
        let parts = text_parts(&req);
        assert_eq!(parts, vec![("site_id", "k7x2qm4j6p".into())]);
    }

    #[test]
    fn build_form_accepts_empty_archive_and_no_metadata() {
        let form = build_form(Vec::new(), "site.zip".into(), &DeployRequest::default())
            .expect("build_form");
        assert!(!form.boundary().is_empty());
    }

    #[test]
    fn build_form_accepts_archive_and_all_fields() {
        let bytes = b"PK\x03\x04fake-zip".to_vec();
        let form =
            build_form(bytes, "site.zip".into(), &req_with_all_fields()).expect("build_form");
        assert!(!form.boundary().is_empty());
    }

    #[test]
    fn deploy_response_deserializes_full_payload() {
        let raw = r#"{
            "id": "k7x2qm4j6p",
            "url": "https://k7x2qm4j6p.a.ls",
            "projectName": "fox042",
            "version": "20260510-103022-a3f1c2",
            "expiresAt": "2026-05-17T10:30:22Z",
            "authRequired": false
        }"#;
        let resp: DeployResponse = serde_json::from_str(raw).expect("decode DeployResponse");
        assert_eq!(resp.id, "k7x2qm4j6p");
        assert_eq!(resp.url, "https://k7x2qm4j6p.a.ls");
        assert_eq!(resp.project_name, "fox042");
        assert_eq!(resp.version, "20260510-103022-a3f1c2");
        assert!(!resp.auth_required);
        assert_eq!(resp.expires_at.as_deref(), Some("2026-05-17T10:30:22Z"));
    }

    #[test]
    fn deploy_response_deserializes_null_expires_at() {
        let raw = r#"{
            "id": "abcdefghij",
            "url": "https://abcdefghij.a.ls",
            "projectName": "never-expires",
            "version": "v1",
            "expiresAt": null,
            "authRequired": true
        }"#;
        let resp: DeployResponse = serde_json::from_str(raw).expect("decode DeployResponse");
        assert!(resp.auth_required);
        assert!(resp.expires_at.is_none());
    }
}
