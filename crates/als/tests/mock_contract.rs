//! Mock-server contract-fidelity suite. These tests do *not* drive the
//! `als` binary — they hit the Bun mock directly with raw HTTP and
//! check it matches the documented `cli-server-contract.md` /
//! `api-reference.md` behaviour. They exist so that future changes to
//! the CLI tests (which all run against this mock) can't quietly drift
//! into asserting behaviour that the real server doesn't actually
//! implement.
//!
//! When one of these tests fails, fix the mock — not the test — until
//! the live server's behaviour starts diverging from the doc, at which
//! point both move together.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

mod common;

use common::SERVER;
use reqwest::blocking::{Client, multipart};
use serde_json::{Value, json};
use serial_test::serial;

const DEFAULT_BEARER: &str = "tk_test0001abcdef";

fn http() -> Client {
    Client::new()
}

fn fetch_state() -> Value {
    let url = SERVER.url.join("/__test/state").expect("join state");
    let body: Value = http()
        .get(url)
        .send()
        .expect("GET /__test/state")
        .json()
        .expect("parse state");
    body.get("data").cloned().unwrap_or(body)
}

#[test]
#[serial]
fn deploy_rejects_request_with_no_archive_field() {
    // Contract: `archive` is the only required form field. A request
    // without it must return 400 VALIDATION_ERROR — the live server
    // refuses to allocate or push anything when there is nothing to
    // upload, and the CLI relies on that to surface a clean error
    // when something has gone wrong locally (e.g. an empty zip
    // buffer).
    SERVER.reset();
    let url = SERVER.url.join("/api/deploy").expect("url");
    let form = multipart::Form::new().text("project_name", "x");
    let resp = http()
        .post(url)
        .bearer_auth(DEFAULT_BEARER)
        .multipart(form)
        .send()
        .expect("POST /api/deploy");

    let status = resp.status().as_u16();
    let body: Value = resp.json().expect("envelope json");
    // Per the contract validation errors land at 400 *or* 422 — the
    // CLI's `from_envelope` table treats them interchangeably. The
    // important guarantees are that it isn't 2xx (no silent allocation)
    // and that the envelope code is one of the validation family.
    assert!(
        status == 400 || status == 422,
        "want 400 or 422, got {status}; body: {body}"
    );
    assert_eq!(body["success"].as_bool(), Some(false));
    let code = body["error"]["code"].as_str().expect("error code");
    assert!(
        code == "VALIDATION_ERROR" || code == "INVALID_MULTIPART",
        "expected validation-style code, got {code}; body: {body}"
    );
}

#[test]
#[serial]
fn sites_list_limit_query_is_capped_at_200() {
    // Contract: `limit` is bounded to 200. The server may silently
    // clamp values above the cap (matching the live server's
    // behaviour); the mock must do the same so the CLI can't depend
    // on receiving more rows than the contract allows.
    SERVER.reset();
    let url = SERVER
        .url
        .join("/api/sites?limit=500")
        .expect("url with limit");
    let resp = http()
        .get(url)
        .bearer_auth(DEFAULT_BEARER)
        .send()
        .expect("GET /api/sites");
    assert!(
        resp.status().is_success(),
        "want 2xx, got {}",
        resp.status()
    );
    let body: Value = resp.json().expect("envelope json");
    assert_eq!(
        body["meta"]["limit"].as_u64(),
        Some(200),
        "limit must be clamped to 200; body: {body}"
    );
}

#[test]
#[serial]
fn password_endpoint_rejects_empty_string() {
    // Contract: `password = ""` is invalid — clearing is expressed as
    // `password: null`, and any non-null string must be a real
    // bearer-style password. The mock returns 400 for the empty case
    // so misuse fails loudly instead of writing an empty hash.
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [{
            "id": "aaaaabbbbb",
            "userId": "u_alice",
            "fullDomain": "aaaaabbbbb.a.ls",
            "projectName": "x",
            "description": null,
            "authRequired": false,
            "passwordHash": null,
            "currentVersion": "20260510-120000-aaaaaa",
            "createdAt": "2026-05-01T00:00:00.000Z",
            "updatedAt": "2026-05-01T00:00:00.000Z",
            "expiresAt": null,
            "releasedAt": null,
            "sizeBytes": 1024,
            "versions": [],
        }],
    }));

    let url = SERVER
        .url
        .join("/api/sites/aaaaabbbbb/password")
        .expect("url");
    let resp = http()
        .post(url)
        .bearer_auth(DEFAULT_BEARER)
        .json(&json!({ "password": "" }))
        .send()
        .expect("POST password");
    let status = resp.status().as_u16();
    let body: Value = resp.json().expect("envelope json");
    assert!(
        status == 400 || status == 422,
        "want 400/422, got {status}; body: {body}"
    );
    assert_eq!(body["error"]["code"].as_str(), Some("VALIDATION_ERROR"));

    // Side-effect check: the rejection did not flip the site to
    // password-protected.
    let snap = fetch_state();
    let site = &snap["sites"][0];
    assert_eq!(site["authRequired"].as_bool(), Some(false), "{site}");
    assert!(site["passwordHash"].is_null(), "{site}");
}

#[test]
#[serial]
fn activate_rejects_version_id_from_a_different_site() {
    // Contract: a version id is owned by exactly one site; `activate`
    // must reject an attempt to activate site B's version on site A
    // with 404 (not a silent success, not a 200 with a swapped
    // version, both of which would corrupt history server-side).
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [
            {
                "id": "aaaaabbbbb",
                "userId": "u_alice",
                "fullDomain": "aaaaabbbbb.a.ls",
                "projectName": "alpha",
                "description": null,
                "authRequired": false,
                "passwordHash": null,
                "currentVersion": "v-a",
                "createdAt": "2026-05-01T00:00:00.000Z",
                "updatedAt": "2026-05-01T00:00:00.000Z",
                "expiresAt": null,
                "releasedAt": null,
                "sizeBytes": 1,
                "versions": [{
                    "id": "v-a", "siteId": "aaaaabbbbb",
                    "createdAt": "2026-05-01T00:00:00.000Z",
                    "sizeBytes": 1, "fileCount": 1, "note": null,
                }],
            },
            {
                "id": "cccccddddd",
                "userId": "u_alice",
                "fullDomain": "cccccddddd.a.ls",
                "projectName": "beta",
                "description": null,
                "authRequired": false,
                "passwordHash": null,
                "currentVersion": "v-b",
                "createdAt": "2026-05-01T00:00:00.000Z",
                "updatedAt": "2026-05-01T00:00:00.000Z",
                "expiresAt": null,
                "releasedAt": null,
                "sizeBytes": 1,
                "versions": [{
                    "id": "v-b", "siteId": "cccccddddd",
                    "createdAt": "2026-05-01T00:00:00.000Z",
                    "sizeBytes": 1, "fileCount": 1, "note": null,
                }],
            },
        ],
    }));

    // Cross-site activate: site A, version id belonging to site B.
    let url = SERVER
        .url
        .join("/api/sites/aaaaabbbbb/activate")
        .expect("url");
    let resp = http()
        .post(url)
        .bearer_auth(DEFAULT_BEARER)
        .json(&json!({ "version": "v-b" }))
        .send()
        .expect("POST activate");
    assert_eq!(
        resp.status().as_u16(),
        404,
        "want 404, got {}",
        resp.status()
    );

    // Server state must be unchanged — neither site flipped its
    // currentVersion in the process.
    let snap = fetch_state();
    let site_a = snap["sites"]
        .as_array()
        .and_then(|arr| arr.iter().find(|s| s["id"].as_str() == Some("aaaaabbbbb")))
        .expect("site a");
    assert_eq!(site_a["currentVersion"].as_str(), Some("v-a"));
}

#[test]
#[serial]
fn deploy_with_site_id_for_released_site_returns_404() {
    // Regression guard for the mock-fidelity fix made when the
    // `stale_als_toml_id_after_server_rm_surfaces_clean_error` test
    // was added: a released (rm'd) site is a tombstone, not a target
    // for further uploads. A raw HTTP probe locks this behaviour in
    // independently of the CLI test that originally surfaced it.
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [{
            "id": "aaaaabbbbb",
            "userId": "u_alice",
            "fullDomain": "aaaaabbbbb.a.ls",
            "projectName": "deleted",
            "description": null,
            "authRequired": false,
            "passwordHash": null,
            "currentVersion": "v-1",
            "createdAt": "2026-05-01T00:00:00.000Z",
            "updatedAt": "2026-05-01T00:00:00.000Z",
            "expiresAt": null,
            "releasedAt": "2026-05-02T00:00:00.000Z",
            "sizeBytes": 1,
            "versions": [{
                "id": "v-1", "siteId": "aaaaabbbbb",
                "createdAt": "2026-05-01T00:00:00.000Z",
                "sizeBytes": 1, "fileCount": 1, "note": null,
            }],
        }],
    }));

    let url = SERVER.url.join("/api/deploy").expect("url");
    let form = multipart::Form::new().text("site_id", "aaaaabbbbb").part(
        "archive",
        multipart::Part::bytes(b"PK\x03\x04dummy".to_vec())
            .file_name("site.zip")
            .mime_str("application/zip")
            .expect("mime"),
    );
    let resp = http()
        .post(url)
        .bearer_auth(DEFAULT_BEARER)
        .multipart(form)
        .send()
        .expect("POST /api/deploy");

    let status = resp.status().as_u16();
    let body: Value = resp.json().expect("envelope json");
    assert_eq!(
        status,
        404,
        "want 404, got {status}; body: {body}; state-pre-deploy: {}",
        fetch_state()
    );
    let code = body["error"]["code"].as_str().expect("code");
    assert!(
        code == "site_not_found" || code == "NOT_FOUND",
        "expected site-not-found-style code, got {code}; body: {body}"
    );
}
