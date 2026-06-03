//! E2E tests for `als config get|set|list` — drives the compiled `als`
//! binary through `assert_cmd` against an isolated `HOME` tempdir. The
//! config commands are local-only (no network, no auth), so this binary
//! never references `common::SERVER` and does not spawn the Bun mock.
//!
//! Each test gets its own isolated `HOME` (`cli_with_home()` returns a
//! fresh tempdir), so cases are independent and run in parallel safely —
//! no `#[serial]` is required.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::cli_with_home;
use predicates::prelude::*;

/// Resolve the on-disk config path the `als` binary will read / write
/// given the harness `XDG_CONFIG_HOME` of `$home/.config`.
fn config_file(home: &Path) -> PathBuf {
    home.join(".config").join("als").join("config.toml")
}

fn seed_config(home: &Path, body: &str) {
    let path = config_file(home);
    fs::create_dir_all(path.parent().expect("config dir")).expect("create config dir");
    fs::write(&path, body).expect("write seed config.toml");
}

#[test]
fn config_get_api() {
    let (mut cmd, home) = cli_with_home();
    seed_config(
        &home,
        r#"
[default]
api = "https://api.example.test"
token = "tk_seedseedseed"
"#,
    );

    let output = cmd
        .args(["config", "get", "default.api"])
        .output()
        .expect("spawn als");
    assert!(
        output.status.success(),
        "exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        output.stdout,
        b"https://api.example.test\n",
        "stdout should be exactly the api URL + newline; got {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
}

#[test]
fn config_set_token() {
    let (mut cmd, home) = cli_with_home();
    // Start from the empty config staged by `cli_with_home`.

    cmd.args(["config", "set", "staging.token", "tk_yyy"])
        .assert()
        .success();

    let on_disk = fs::read_to_string(config_file(&home)).expect("read config after set");
    let parsed: toml::Value = toml::from_str(&on_disk).expect("on-disk config parses as TOML");
    let token = parsed
        .get("profile")
        .and_then(|p| p.get("staging"))
        .and_then(|s| s.get("token"))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("expected profile.staging.token in:\n{on_disk}"));
    assert_eq!(token, "tk_yyy");
}

#[test]
fn config_set_invalid_url() {
    let (mut cmd, _home) = cli_with_home();

    cmd.args(["config", "set", "default.api", "not-a-url"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("invalid api URL"));
}

#[test]
fn config_list_masks_token() {
    let (mut cmd, home) = cli_with_home();
    // The masker keeps the first 8 chars + `**`; the body below pads the
    // token past 8 chars so the masked head is `tk_xxxxx` and the
    // `_secret_value` tail is provably absent from stdout.
    seed_config(
        &home,
        r#"
[default]
api = "https://api.example.test"
token = "tk_xxxxxxxx_secret_value"
"#,
    );

    let output = cmd.args(["config", "list"]).output().expect("spawn als");
    assert!(
        output.status.success(),
        "exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    assert!(
        stdout.contains("tk_xxxxx**"),
        "stdout should contain the masked token head + `**`:\n{stdout}",
    );
    assert!(
        !stdout.contains("_secret_value"),
        "stdout must not leak the post-prefix portion of the token:\n{stdout}",
    );
}
