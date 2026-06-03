//! `GET quota/me` — per-user quota and usage counters.
//!
//! Consumed by `als auth status` to render the quota footer. The
//! server returns a richer payload (notify-* preferences, etc.); the
//! CLI only models the fields it surfaces and ignores the rest.

use serde::{Deserialize, Serialize};

use crate::Client;
use crate::error::ApiError;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    /// `None` (JSON `null`) or `Some(0)` both mean "unlimited" — the
    /// server uses null for accounts without a hard cap, and historically
    /// some payloads sent 0 to mean the same thing. Any positive integer
    /// is the real ceiling.
    #[serde(default)]
    pub max_sites: Option<u32>,
    pub max_size_bytes: u64,
    #[serde(default)]
    pub max_extracted_bytes: Option<u64>,
    #[serde(default)]
    pub max_files_per_site: Option<u32>,
    pub total_sites: u32,
    pub total_bytes: u64,
}

impl Quota {
    /// Returns the site cap as a finite positive integer, or `None` when
    /// the account is unlimited (server sent `null` or `0`).
    #[must_use]
    pub fn site_cap(&self) -> Option<u32> {
        self.max_sites.filter(|&n| n > 0)
    }
}

pub async fn me(client: &Client) -> Result<Quota, ApiError> {
    client.get_json("quota/me", &[]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_deserializes_typical_payload() {
        let body = r#"{
            "userId": "u_alice",
            "maxSites": 100,
            "maxSizeBytes": 52428800,
            "maxExtractedBytes": 209715200,
            "maxFilesPerSite": 1000,
            "totalSites": 3,
            "totalBytes": 1024,
            "notifyEmail": null,
            "notifyOnExpire": true,
            "notifyOnQuota": true
        }"#;
        let q: Quota = serde_json::from_str(body).unwrap();
        assert_eq!(q.max_sites, Some(100));
        assert_eq!(q.site_cap(), Some(100));
        assert_eq!(q.max_size_bytes, 52_428_800);
        assert_eq!(q.total_sites, 3);
        assert_eq!(q.total_bytes, 1024);
        assert_eq!(q.max_extracted_bytes, Some(209_715_200));
    }

    #[test]
    fn quota_tolerates_missing_optionals() {
        let body = r#"{
            "maxSites": 50,
            "maxSizeBytes": 10485760,
            "totalSites": 0,
            "totalBytes": 0
        }"#;
        let q: Quota = serde_json::from_str(body).unwrap();
        assert!(q.max_extracted_bytes.is_none());
    }

    #[test]
    fn quota_treats_null_max_sites_as_unlimited() {
        let body = r#"{
            "maxSites": null,
            "maxSizeBytes": 10485760,
            "totalSites": 0,
            "totalBytes": 0
        }"#;
        let q: Quota = serde_json::from_str(body).unwrap();
        assert_eq!(q.max_sites, None);
        assert_eq!(q.site_cap(), None);
    }

    #[test]
    fn quota_treats_zero_max_sites_as_unlimited() {
        let body = r#"{
            "maxSites": 0,
            "maxSizeBytes": 10485760,
            "totalSites": 0,
            "totalBytes": 0
        }"#;
        let q: Quota = serde_json::from_str(body).unwrap();
        assert_eq!(q.max_sites, Some(0));
        assert_eq!(q.site_cap(), None);
    }

    #[test]
    fn quota_tolerates_omitted_max_sites() {
        let body = r#"{
            "maxSizeBytes": 10485760,
            "totalSites": 0,
            "totalBytes": 0
        }"#;
        let q: Quota = serde_json::from_str(body).unwrap();
        assert_eq!(q.max_sites, None);
        assert_eq!(q.site_cap(), None);
    }
}
