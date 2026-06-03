//! E2E for `als <code>` and `als --kind code` modes. Drives the
//! compiled binary against the Bun mock server like every other E2E
//! file, and asserts only what is visible at the CLI boundary (exit
//! code + stdout / stderr lines + that the mock recorded one upload).
//!
//! Structural assertions on the bundle (manifest contents, CDN URL,
//! raw/ layout) live in `crates/als-code/src/**` unit tests
//! against `als_code::bundle` directly.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use std::fs;

use common::{SERVER, cli};
use predicates::prelude::*;
use serde_json::Value;
use serial_test::serial;
use tempfile::TempDir;

fn code_dir() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("src")).expect("mkdir src");
    fs::write(dir.path().join("Cargo.toml"), b"[package]\nname = \"x\"\n").expect("Cargo.toml");
    fs::write(dir.path().join("src/main.rs"), b"fn main() {}\n").expect("main.rs");
    dir
}

fn single_code_file() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("snippet.ts");
    fs::write(&file, b"export const answer = 42;\n").expect("snippet.ts");
    (dir, file)
}

fn fetch_state() -> Value {
    let url = SERVER.url.join("/__test/state").expect("join state");
    let body: Value = reqwest::blocking::Client::new()
        .get(url)
        .send()
        .expect("GET state")
        .json()
        .expect("parse state json");
    body.get("data").cloned().unwrap_or(body)
}

#[test]
#[serial]
fn deploy_single_code_file_succeeds() {
    SERVER.reset();
    let (_dir, file) = single_code_file();
    let assert = cli().arg(&file).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("Deployed:") && stdout.contains(".a.ls"),
        "deploy stdout missing url:\n{stdout}",
    );
    let snap = fetch_state();
    assert_eq!(snap["sites"].as_array().map(Vec::len), Some(1));
}

#[test]
#[serial]
fn deploy_code_directory_succeeds() {
    SERVER.reset();
    let dir = code_dir();
    cli().arg(dir.path()).assert().success();
    let snap = fetch_state();
    assert_eq!(snap["sites"].as_array().map(Vec::len), Some(1));
}

#[test]
#[serial]
fn deploy_with_kind_site_on_code_dir_still_uploads() {
    // `--kind site` skips the code pipeline; the directory has no
    // index.html so it uploads as a plain zip. The server doesn't care.
    SERVER.reset();
    let dir = code_dir();
    cli()
        .arg(dir.path())
        .args(["--kind", "site"])
        .assert()
        .success();
    let snap = fetch_state();
    assert_eq!(snap["sites"].as_array().map(Vec::len), Some(1));
}

#[test]
#[serial]
fn deploy_with_kind_code_on_html_dir_succeeds() {
    // `--kind code` forces the code bundle even when the dir
    // would normally route to plain site (has index.html). The original
    // index.html ends up as `raw/index.html` inside the bundle.
    SERVER.reset();
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();
    fs::write(dir.path().join("main.ts"), b"export {};\n").unwrap();
    cli()
        .arg(dir.path())
        .args(["--kind", "code"])
        .assert()
        .success();
    let snap = fetch_state();
    assert_eq!(snap["sites"].as_array().map(Vec::len), Some(1));
}

#[test]
#[serial]
fn kind_code_on_empty_dir_reports_no_files() {
    // Forced `--kind code` skips the html / md shape check but the
    // walker still produces no entries on an empty dir, so the user
    // sees `code_no_files` with a useful hint.
    SERVER.reset();
    let dir = tempfile::tempdir().expect("tempdir");
    let assert = cli()
        .arg(dir.path())
        .args(["--kind", "code"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("code_no_files"),
        "expected code_no_files in stderr, got:\n{stderr}",
    );
}

#[test]
#[serial]
fn too_many_files_errors_with_documented_prefix() {
    SERVER.reset();
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..=als_code::MAX_FILES {
        fs::write(dir.path().join(format!("f{i}.rs")), b"//\n").unwrap();
    }
    let assert = cli().arg(dir.path()).assert().failure();
    assert.stderr(predicate::str::contains("code_too_many_files"));
}

#[test]
#[serial]
fn json_mode_still_emits_site_id() {
    SERVER.reset();
    let (_dir, file) = single_code_file();
    let assert = cli().arg(&file).arg("--json").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: Value = serde_json::from_str(&stdout).expect("parse json");
    assert!(value.get("id").is_some(), "missing id in json: {stdout}");
    assert!(value.get("url").is_some(), "missing url in json: {stdout}");
}
