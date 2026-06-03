//! E2E tests for `als <path>` — drive the compiled `als` binary
//! through `assert_cmd` against the Bun mock server. Each test resets
//! the mock and is `#[serial]` per the harness rules in
//! `tests/common/mod.rs`.

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

fn dir_with_one_file() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>hi</title>",
    )
    .expect("write index.html");
    dir
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
fn deploy_round_trip_emits_url() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let assert = cli().arg(dir.path()).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("Deployed:") && stdout.contains(".a.ls"),
        "deploy stdout missing url:\n{stdout}"
    );
    let snap = fetch_state();
    assert_eq!(
        snap["sites"].as_array().map(Vec::len),
        Some(1),
        "mock should have one site"
    );
}

#[test]
#[serial]
fn deploy_quiet_emits_only_url() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let assert = cli().arg(dir.path()).arg("--quiet").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // exactly one line ending with .a.ls/
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "expected one stdout line, got: {stdout:?}");
    assert!(
        lines[0].starts_with("https://") && lines[0].contains(".a.ls"),
        "expected url-only stdout, got: {}",
        lines[0]
    );
}

#[test]
#[serial]
fn deploy_with_pass_auto_prints_generated_password() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let assert = cli()
        .arg(dir.path())
        .args(["--pass", "auto"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("Password: "),
        "expected generated password line:\n{stdout}"
    );
    let snap = fetch_state();
    let site = &snap["sites"][0];
    assert_eq!(site["authRequired"].as_bool(), Some(true));
}

#[test]
#[serial]
fn deploy_with_json_emits_id_and_url() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let assert = cli().arg(dir.path()).arg("--json").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: Value = serde_json::from_str(&stdout).expect("json mode emits json");
    assert!(parsed["id"].as_str().is_some(), "{}", parsed);
    assert!(parsed["url"].as_str().is_some(), "{}", parsed);
}

// --- Markdown / mdbook pre-render --------------------------------------------

#[test]
#[serial]
fn deploy_single_markdown_renders_and_uploads() {
    SERVER.reset();
    let dir = tempfile::tempdir().expect("tempdir");
    let md = dir.path().join("notes.md");
    fs::write(&md, "# Notes\n\nHello world.\n").expect("write md");

    cli()
        .arg(&md)
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("https://"));
}

#[test]
#[serial]
fn deploy_markdown_directory_renders_and_uploads() {
    SERVER.reset();
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("index.md"),
        "# Home\n\nSee [next](other.md).\n",
    )
    .expect("write index.md");
    fs::write(
        dir.path().join("other.md"),
        "# Other\n\nBack to [home](index.md).\n",
    )
    .expect("write other.md");

    cli()
        .arg(dir.path())
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("https://"));
}

#[test]
#[serial]
fn deploy_mdbook_book_renders_and_uploads() {
    SERVER.reset();
    let book = tempfile::tempdir().expect("tempdir");
    fs::write(
        book.path().join("book.toml"),
        "[book]\ntitle = \"Test Book\"\nauthors = [\"e2e\"]\n",
    )
    .expect("write book.toml");
    let src = book.path().join("src");
    fs::create_dir_all(&src).expect("mkdir src");
    fs::write(src.join("SUMMARY.md"), "# Summary\n\n- [Intro](intro.md)\n")
        .expect("write SUMMARY.md");
    fs::write(src.join("intro.md"), "# Intro\n\nWelcome.\n").expect("write intro.md");

    cli()
        .arg(book.path())
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("https://"));
}
