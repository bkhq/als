//! Integration tests for `endpoints::sites` against the Bun mock server.
//!
//! Each test resets the shared mock, seeds two sites for the default user
//! (token `tk_test0001abcdef`), drives the typed client, and asserts the
//! observable shape. `serial_test::serial` is mandatory: the mock state
//! is process-global.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

mod common;

use std::path::PathBuf;

use als_api::{ApiError, Client, ListSitesQuery, PatchSiteRequest, delete, get, list, patch};
use common::SERVER;
use secrecy::SecretString;
use serde_json::{Value, json};
use serial_test::serial;

const TEST_TOKEN: &str = "tk_test0001abcdef";
const USER_ID: &str = "u_alice";

fn client() -> Client {
    let cfg = als_core::Config {
        api: SERVER.url.clone(),
        token: SecretString::from(TEST_TOKEN.to_owned()),
        active_profile: None,
        source_path: PathBuf::from("/tmp/test.toml"),
    };
    Client::new(&cfg).expect("client builds")
}

async fn seed_two_sites() {
    tokio::task::spawn_blocking(|| {
        SERVER.reset();
        SERVER.seed(two_sites_fixture());
    })
    .await
    .expect("blocking seed task");
}

fn two_sites_fixture() -> Value {
    json!({
        "sites": [
            {
                "id": "aaaaabbbbb",
                "userId": USER_ID,
                "fullDomain": "aaaaabbbbb.a.ls",
                "projectName": "alpha",
                "description": null,
                "authRequired": false,
                "passwordHash": null,
                "currentVersion": "20260510-120000-aaaaaa",
                "createdAt": "2026-05-01T00:00:00.000Z",
                "updatedAt": "2026-05-01T00:00:00.000Z",
                "expiresAt": "2026-05-16T00:00:00.000Z",
                "releasedAt": null,
                "sizeBytes": 1024,
                "versions": [
                    {
                        "id": "20260510-120000-aaaaaa",
                        "siteId": "aaaaabbbbb",
                        "createdAt": "2026-05-01T00:00:00.000Z",
                        "sizeBytes": 1024,
                        "fileCount": 3,
                        "note": null
                    }
                ]
            },
            {
                "id": "bbbbbccccc",
                "userId": USER_ID,
                "fullDomain": "bbbbbccccc.a.ls",
                "projectName": "bravo",
                "description": "second site",
                "authRequired": true,
                "passwordHash": "argon2id$dummy",
                "currentVersion": "20260510-130000-bbbbbb",
                "createdAt": "2026-05-02T00:00:00.000Z",
                "updatedAt": "2026-05-02T00:00:00.000Z",
                "expiresAt": null,
                "releasedAt": null,
                "sizeBytes": 2048,
                "versions": [
                    {
                        "id": "20260510-130000-bbbbbb",
                        "siteId": "bbbbbccccc",
                        "createdAt": "2026-05-02T00:00:00.000Z",
                        "sizeBytes": 2048,
                        "fileCount": 5,
                        "note": "deploy two"
                    }
                ]
            }
        ]
    })
}

#[tokio::test]
#[serial]
async fn list_returns_both_sites_with_pagination_meta() {
    seed_two_sites().await;
    let resp = list(&client(), &ListSitesQuery::default())
        .await
        .expect("list");
    assert_eq!(resp.data.len(), 2);
    assert_eq!(resp.meta.total, 2);
    assert_eq!(resp.meta.page, 1);
    assert_eq!(resp.meta.limit, 50);
}

#[tokio::test]
#[serial]
async fn get_returns_full_site_row() {
    seed_two_sites().await;
    let site = get(&client(), "aaaaabbbbb").await.expect("get");
    assert_eq!(site.id, "aaaaabbbbb");
    assert_eq!(site.project_name, "alpha");
    assert!(!site.auth_required);
    assert!(site.expires_at.is_some());
}

#[tokio::test]
#[serial]
async fn get_missing_returns_not_found() {
    seed_two_sites().await;
    let err = get(&client(), "zzzzz").await.unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}

#[tokio::test]
#[serial]
async fn patch_updates_project_name() {
    seed_two_sites().await;
    let body = PatchSiteRequest {
        project_name: Some("renamed".into()),
        ..PatchSiteRequest::default()
    };
    let site = patch(&client(), "aaaaabbbbb", &body).await.expect("patch");
    assert_eq!(site.project_name, "renamed");
}

#[tokio::test]
#[serial]
async fn patch_clears_expires_with_explicit_null() {
    seed_two_sites().await;
    let body = PatchSiteRequest {
        expires_at: Some(None),
        ..PatchSiteRequest::default()
    };
    let site = patch(&client(), "aaaaabbbbb", &body).await.expect("patch");
    assert!(site.expires_at.is_none());
}

#[tokio::test]
#[serial]
async fn delete_releases_site_then_returns_not_found_on_get() {
    seed_two_sites().await;
    delete(&client(), "aaaaabbbbb").await.expect("delete");
    let err = get(&client(), "aaaaabbbbb").await.unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}
