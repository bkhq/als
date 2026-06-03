//! `sites` and `sites/:id` endpoints.
//!
//! Mirrors the wire shapes exposed by the live als server under
//! `GET /api/sites`, `GET /api/sites/:id`, `PATCH /api/sites/:id`, and
//! `DELETE /api/sites/:id`. Password and version sub-resources live in
//! `password.rs` / `versions.rs`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Client;
use crate::envelope::PaginationMeta;
use crate::error::ApiError;

/// One row from `GET /api/sites` (and the body of `GET /api/sites/:id`).
///
/// Wire format is camelCase; the Rust struct uses `snake_case` names
/// plus a single `rename_all` attribute to bridge the two.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Site {
    pub id: String,
    pub full_domain: String,
    pub project_name: String,
    pub description: Option<String>,
    pub auth_required: bool,
    pub current_version: String,
    /// ISO-8601 timestamp.
    pub created_at: String,
    /// ISO-8601 timestamp.
    pub updated_at: String,
    /// ISO-8601 timestamp, or `null` for "never expires".
    pub expires_at: Option<String>,
    /// Populated only after the site is soft-deleted.
    pub released_at: Option<String>,
}

/// Site lifecycle state. The server does not embed this in the row —
/// the binary derives it from `expires_at` / `released_at` for
/// human-friendly listing filters. `All` is a filter sentinel.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SiteState {
    Active,
    Expired,
    Released,
    All,
}

impl SiteState {
    /// Borrow the snake-case wire identifier without going through the
    /// `Display` formatter (avoids the heap allocation that
    /// `format!("{state}")` introduces).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Released => "released",
            Self::All => "all",
        }
    }

    /// Derive a site's state from its `released_at` / `expires_at`
    /// against the supplied current ISO timestamp. Used by `als list
    /// --state` to post-filter the server response because the server
    /// does not expose a `state` query parameter.
    #[must_use]
    pub fn derive(site: &Site, now_iso: &str) -> Self {
        if site.released_at.is_some() {
            return Self::Released;
        }
        match &site.expires_at {
            Some(exp) if exp.as_str() <= now_iso => Self::Expired,
            _ => Self::Active,
        }
    }
}

impl fmt::Display for SiteState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Filter / pagination args for `GET /api/sites`. The server caps `limit`
/// at 200; the CLI doesn't enforce that — the server is authoritative.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ListSitesQuery {
    pub limit: Option<u32>,
    pub page: Option<u32>,
    /// Admin-only: return every active site, not just the caller's. The
    /// server silently ignores this for non-admins.
    pub all: bool,
}

/// Server response for `GET /api/sites` after envelope parsing.
#[derive(Debug, Clone)]
pub struct ListSitesResponse {
    pub data: Vec<Site>,
    pub meta: PaginationMeta,
}

/// PATCH body for `sites/:id`. All fields are optional; the server keeps
/// any omitted field unchanged. `Option<Option<T>>` distinguishes "leave
/// alone" (`None`) from "clear" (`Some(None)`).
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchSiteRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Option<String>>,
}

/// `GET /api/sites?page=…&limit=…&all=…`.
pub async fn list(client: &Client, q: &ListSitesQuery) -> Result<ListSitesResponse, ApiError> {
    let page_buf;
    let limit_buf;
    let mut query: Vec<(&str, &str)> = Vec::with_capacity(3);
    if let Some(page) = q.page {
        page_buf = page.to_string();
        query.push(("page", &page_buf));
    }
    if let Some(limit) = q.limit {
        limit_buf = limit.to_string();
        query.push(("limit", &limit_buf));
    }
    if q.all {
        query.push(("all", "true"));
    }
    let payload = client.get_list::<Site>("sites", &query).await?;
    Ok(ListSitesResponse {
        data: payload.data,
        meta: payload.meta,
    })
}

/// `GET /api/sites/:id`.
pub async fn get(client: &Client, id: &str) -> Result<Site, ApiError> {
    let path = format!("sites/{id}");
    client.get_json(&path, &[]).await
}

/// `DELETE /api/sites/:id`. Server returns 200 `{success:true,data:null}`.
pub async fn delete(client: &Client, id: &str) -> Result<(), ApiError> {
    let path = format!("sites/{id}");
    client.delete(&path).await
}

/// `PATCH /api/sites/:id` — partial update. Returns the full updated row.
pub async fn patch(client: &Client, id: &str, body: &PatchSiteRequest) -> Result<Site, ApiError> {
    let path = format!("sites/{id}");
    client.patch_json(&path, body).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_site(expires: Option<&str>, released: Option<&str>) -> Site {
        Site {
            id: "k7x2qm4j6p".into(),
            full_domain: "k7x2qm4j6p.a.ls".into(),
            project_name: "demo".into(),
            description: None,
            auth_required: false,
            current_version: "v1".into(),
            created_at: "2026-05-01T00:00:00.000Z".into(),
            updated_at: "2026-05-01T00:00:00.000Z".into(),
            expires_at: expires.map(str::to_owned),
            released_at: released.map(str::to_owned),
        }
    }

    #[test]
    fn site_state_display_matches_wire() {
        assert_eq!(SiteState::Active.to_string(), "active");
        assert_eq!(SiteState::Expired.to_string(), "expired");
        assert_eq!(SiteState::Released.to_string(), "released");
        assert_eq!(SiteState::All.to_string(), "all");
    }

    #[test]
    fn site_state_serde_round_trip() {
        let json = serde_json::to_string(&SiteState::Released).unwrap();
        assert_eq!(json, r#""released""#);
        let back: SiteState = serde_json::from_str(r#""expired""#).unwrap();
        assert_eq!(back, SiteState::Expired);
    }

    #[test]
    fn site_state_derive_released_wins() {
        let site = sample_site(Some("2027-01-01T00:00:00Z"), Some("2026-05-09T00:00:00Z"));
        let state = SiteState::derive(&site, "2026-05-10T00:00:00Z");
        assert_eq!(state, SiteState::Released);
    }

    #[test]
    fn site_state_derive_expired_when_past() {
        let site = sample_site(Some("2026-05-09T00:00:00Z"), None);
        let state = SiteState::derive(&site, "2026-05-10T00:00:00Z");
        assert_eq!(state, SiteState::Expired);
    }

    #[test]
    fn site_state_derive_active_when_future() {
        let site = sample_site(Some("2027-01-01T00:00:00Z"), None);
        let state = SiteState::derive(&site, "2026-05-10T00:00:00Z");
        assert_eq!(state, SiteState::Active);
    }

    #[test]
    fn site_state_derive_active_when_no_expiry() {
        let site = sample_site(None, None);
        let state = SiteState::derive(&site, "2026-05-10T00:00:00Z");
        assert_eq!(state, SiteState::Active);
    }

    #[test]
    fn site_deserializes_full_payload() {
        let body = r#"{
            "id": "k7x2qm4j6p",
            "fullDomain": "k7x2qm4j6p.a.ls",
            "projectName": "fox042",
            "description": null,
            "authRequired": false,
            "currentVersion": "20260510-103022-a3f1c2",
            "createdAt": "2026-05-09T10:30:22.000Z",
            "updatedAt": "2026-05-09T10:30:22.000Z",
            "expiresAt": "2026-05-16T10:30:22.000Z",
            "releasedAt": null
        }"#;
        let site: Site = serde_json::from_str(body).unwrap();
        assert_eq!(site.id, "k7x2qm4j6p");
        assert_eq!(site.full_domain, "k7x2qm4j6p.a.ls");
        assert!(site.expires_at.is_some());
        assert!(site.released_at.is_none());
    }

    #[test]
    fn patch_skips_unset_fields() {
        let body = PatchSiteRequest {
            project_name: Some("New Name".into()),
            description: None,
            expires_at: None,
        };
        let s = serde_json::to_string(&body).unwrap();
        assert_eq!(s, r#"{"projectName":"New Name"}"#);
    }

    #[test]
    fn patch_clears_expires_with_null() {
        let body = PatchSiteRequest {
            project_name: None,
            description: None,
            expires_at: Some(None),
        };
        let s = serde_json::to_string(&body).unwrap();
        assert_eq!(s, r#"{"expiresAt":null}"#);
    }

    #[test]
    fn patch_sets_expires_with_iso_string() {
        let body = PatchSiteRequest {
            project_name: None,
            description: None,
            expires_at: Some(Some("2026-12-31T23:59:59Z".into())),
        };
        let s = serde_json::to_string(&body).unwrap();
        assert_eq!(s, r#"{"expiresAt":"2026-12-31T23:59:59Z"}"#);
    }

    #[test]
    fn patch_clears_description_with_null() {
        let body = PatchSiteRequest {
            project_name: None,
            description: Some(None),
            expires_at: None,
        };
        let s = serde_json::to_string(&body).unwrap();
        assert_eq!(s, r#"{"description":null}"#);
    }
}
