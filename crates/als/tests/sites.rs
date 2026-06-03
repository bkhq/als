//! E2E suite for `als list` / `als site` / `als rm`.
//!
//! Drives the real `als` binary via `assert_cmd` against the Bun mock
//! server. Every test resets the mock first. `#[serial]` is mandatory
//! because the shared `SERVER` singleton holds process-global state.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

mod common;

use common::{SERVER, cli};
use serde_json::{Value, json};
use serial_test::serial;

const USER_ID: &str = "u_alice";

fn site_fixture(id: &str, project: &str, auth_required: bool, expires: Option<&str>) -> Value {
    json!({
        "id": id,
        "userId": USER_ID,
        "fullDomain": format!("{id}.a.ls"),
        "projectName": project,
        "description": null,
        "authRequired": auth_required,
        "passwordHash": if auth_required { Value::String("hash".into()) } else { Value::Null },
        "currentVersion": "20260510-120000-aaaaaa",
        "createdAt": "2026-05-01T00:00:00.000Z",
        "updatedAt": "2026-05-01T00:00:00.000Z",
        "expiresAt": expires.map_or(Value::Null, Value::from),
        "releasedAt": null,
        "sizeBytes": 1024,
        "versions": [{
            "id": "20260510-120000-aaaaaa",
            "siteId": id,
            "createdAt": "2026-05-01T00:00:00.000Z",
            "sizeBytes": 1024,
            "fileCount": 1,
            "note": null,
        }],
    })
}

fn fetch_state() -> Value {
    let url = SERVER
        .url
        .join("/__test/state")
        .expect("join state endpoint");
    let body: Value = reqwest::blocking::Client::new()
        .get(url)
        .send()
        .expect("GET /__test/state")
        .json()
        .expect("parse state json");
    body.get("data").cloned().unwrap_or(body)
}

fn stdout_string(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout")
}

#[test]
#[serial]
fn list_empty() {
    SERVER.reset();
    let assert = cli().arg("list").assert().success();
    let stdout = stdout_string(&assert);
    assert!(
        stdout.contains("0 sites"),
        "expected '0 sites' summary:\n{stdout}"
    );
}

#[test]
#[serial]
fn list_renders_two_sites() {
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [
            site_fixture("aaaaabbbbb", "alpha", false, Some("2099-01-01T00:00:00Z")),
            site_fixture("bbbbbccccc", "bravo", true, Some("2099-01-01T00:00:00Z")),
        ]
    }));
    let assert = cli().arg("list").assert().success();
    let stdout = stdout_string(&assert);
    assert!(stdout.contains("alpha"), "missing alpha:\n{stdout}");
    assert!(stdout.contains("bravo"), "missing bravo:\n{stdout}");
    assert!(
        stdout.contains("2 sites"),
        "expected '2 sites' summary:\n{stdout}"
    );
}

#[test]
#[serial]
fn site_detail_shows_version_history() {
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [site_fixture("aaaaabbbbb", "docs", false, Some("2099-01-01T00:00:00Z"))]
    }));
    let assert = cli().args(["site", "aaaaabbbbb"]).assert().success();
    let stdout = stdout_string(&assert);
    assert!(stdout.contains("docs"), "missing project name:\n{stdout}");
    assert!(
        stdout.contains("20260510-120000-aaaaaa"),
        "missing version id:\n{stdout}"
    );
    assert!(
        stdout.contains("Versions"),
        "missing 'Versions' section:\n{stdout}"
    );
}

#[test]
#[serial]
fn site_resolves_by_name() {
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [site_fixture("aaaaabbbbb", "docs", false, Some("2099-01-01T00:00:00Z"))]
    }));
    let assert = cli().args(["site", "docs"]).assert().success();
    let stdout = stdout_string(&assert);
    assert!(
        stdout.contains("aaaaabbbbb.a.ls"),
        "name lookup did not reach the right site:\n{stdout}"
    );
}

#[test]
#[serial]
fn site_ambiguous_name_exits_one() {
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [
            site_fixture("aaaaabbbbb", "docs", false, Some("2099-01-01T00:00:00Z")),
            site_fixture("bbbbbccccc", "docs", false, Some("2099-01-01T00:00:00Z")),
        ]
    }));
    let assert = cli().args(["site", "docs"]).assert().failure().code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("ambiguous"),
        "expected ambiguity error:\n{stderr}"
    );
}

#[test]
#[serial]
fn site_rename_via_name_flag() {
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [site_fixture("aaaaabbbbb", "old", false, Some("2099-01-01T00:00:00Z"))]
    }));
    cli()
        .args(["site", "aaaaabbbbb", "--name", "renamed"])
        .assert()
        .success();
    let snap = fetch_state();
    let site = &snap["sites"][0];
    assert_eq!(site["projectName"].as_str(), Some("renamed"));
}

#[test]
#[serial]
fn site_no_pass_clears_password() {
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [site_fixture("aaaaabbbbb", "secret", true, Some("2099-01-01T00:00:00Z"))]
    }));
    cli()
        .args(["site", "aaaaabbbbb", "--no-pass", "-y"])
        .assert()
        .success();
    let snap = fetch_state();
    let site = &snap["sites"][0];
    assert_eq!(site["authRequired"].as_bool(), Some(false));
    assert!(site["passwordHash"].is_null(), "{site}");
}

#[test]
#[serial]
fn rm_releases_single_site() {
    SERVER.reset();
    SERVER.seed(json!({
        "sites": [site_fixture("aaaaabbbbb", "x", false, Some("2099-01-01T00:00:00Z"))]
    }));
    cli().args(["rm", "aaaaabbbbb", "-y"]).assert().success();
    let snap = fetch_state();
    assert!(!snap["sites"][0]["releasedAt"].is_null(), "{}", snap);
}
