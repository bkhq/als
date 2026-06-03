//! E2E coverage for `als site <id> --version <vid>` (rollback /
//! roll-forward) and the implicit `GET /api/sites/:id/versions` call
//! that backs `als site <id>`'s detail view.
//!
//! Drives the real `als` binary via `assert_cmd` against the Bun mock
//! server. Every test resets the mock first; `#[serial]` is mandatory
//! because the `SERVER` singleton holds process-global state.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

mod common;

use std::fs;

use common::{SERVER, cli};
use serde_json::Value;
use serial_test::serial;
use tempfile::TempDir;

fn dir_with_one_file() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>v1</title>",
    )
    .expect("write v1");
    dir
}

fn fetch_state() -> Value {
    let url = SERVER.url.join("/__test/state").expect("join state");
    let body: Value = reqwest::blocking::Client::new()
        .get(url)
        .send()
        .expect("GET /__test/state")
        .json()
        .expect("parse state");
    body.get("data").cloned().unwrap_or(body)
}

fn stdout_of(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout")
}

/// Find the unique site in `fetch_state()` and read its
/// `currentVersion` field.
fn current_version(site_id: &str) -> String {
    let snap = fetch_state();
    let site = snap["sites"]
        .as_array()
        .and_then(|arr| arr.iter().find(|s| s["id"].as_str() == Some(site_id)))
        .unwrap_or_else(|| panic!("no site {site_id} in snapshot: {snap}"));
    site["currentVersion"].as_str().map_or_else(
        || panic!("missing currentVersion on site: {site}"),
        str::to_owned,
    )
}

fn version_ids(site_id: &str) -> Vec<String> {
    let snap = fetch_state();
    let site = snap["sites"]
        .as_array()
        .and_then(|arr| arr.iter().find(|s| s["id"].as_str() == Some(site_id)))
        .unwrap_or_else(|| panic!("no site {site_id} in snapshot: {snap}"));
    site["versions"]
        .as_array()
        .unwrap_or_else(|| panic!("no versions array: {site}"))
        .iter()
        .map(|v| v["id"].as_str().expect("version id").to_owned())
        .collect()
}

fn extract_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| {
            line.split_whitespace()
                .find(|tok| tok.starts_with("https://"))
        })
        .map(str::to_owned)
}

fn site_id_from_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    let id = host.split('.').next()?;
    Some(id.to_owned())
}

#[test]
#[serial]
fn activate_past_version_flips_server_current_version() {
    // The full rollback flow: deploy two distinct versions, run
    // `als site <id> --version <v1-id> -y`, observe that the server
    // updates `currentVersion` to the older one.
    SERVER.reset();
    let dir = dir_with_one_file();

    // v1.
    let r1 = cli().arg(dir.path()).assert().success();
    let url = extract_url(&stdout_of(&r1)).expect("v1 URL");
    let site_id = site_id_from_url(&url).expect("site id");
    let v1 = current_version(&site_id);

    // v2 — content change forces a real deploy (no skip).
    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>v2</title>",
    )
    .expect("rewrite to v2");
    cli().arg(dir.path()).assert().success();
    let v2 = current_version(&site_id);
    assert_ne!(v1, v2, "v1 and v2 must be distinct version ids");

    // Roll back to v1.
    cli()
        .args(["site", &site_id, "--version", &v1, "-y"])
        .assert()
        .success();

    assert_eq!(
        current_version(&site_id),
        v1,
        "server should mark v1 active after activate; versions: {:?}",
        version_ids(&site_id)
    );
}

#[test]
#[serial]
fn activate_unknown_version_exits_one_and_leaves_state_unchanged() {
    // Server returns 404 not_found when the version id is unknown;
    // the CLI maps that onto exit 1 with a clean error message and
    // does not mutate `currentVersion` server-side.
    SERVER.reset();
    let dir = dir_with_one_file();
    let r1 = cli().arg(dir.path()).assert().success();
    let url = extract_url(&stdout_of(&r1)).expect("URL");
    let site_id = site_id_from_url(&url).expect("id");
    let before = current_version(&site_id);

    let assert = cli()
        .args(["site", &site_id, "--version", "does-not-exist", "-y"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    let stdout = stdout_of(&assert);
    let combined = format!("{stdout}\n{stderr}").to_lowercase();
    assert!(
        combined.contains("not found") || combined.contains("version"),
        "expected not-found-style message, got stdout=\n{stdout}\nstderr=\n{stderr}"
    );

    assert_eq!(
        current_version(&site_id),
        before,
        "server-side state must be untouched on a failed activate"
    );
}

#[test]
#[serial]
fn detail_view_lists_every_pushed_version() {
    // `als site <id>` (no edit flag) calls GET /sites/:id +
    // GET /sites/:id/versions. The Human render must surface every
    // version the server holds — the contract `versions.rs` route
    // sorts newest-first; the CLI is expected to preserve that order.
    SERVER.reset();
    let dir = dir_with_one_file();
    cli().arg(dir.path()).assert().success();

    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>v2</title>",
    )
    .expect("v2");
    cli().arg(dir.path()).assert().success();

    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>v3</title>",
    )
    .expect("v3");
    let r3 = cli().arg(dir.path()).assert().success();
    let url = extract_url(&stdout_of(&r3)).expect("URL");
    let site_id = site_id_from_url(&url).expect("id");

    let detail = cli().args(["site", &site_id]).assert().success();
    let stdout = stdout_of(&detail);

    let versions = version_ids(&site_id);
    assert_eq!(versions.len(), 3, "expected 3 versions; got {versions:?}");
    for v in &versions {
        assert!(
            stdout.contains(v),
            "detail output missing version {v}:\n{stdout}"
        );
    }
}
