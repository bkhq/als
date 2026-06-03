//! E2E tests for `als auth login` — drive the compiled `als` binary
//! through the RFC 8628 device-flow against the Bun mock server. Each
//! test resets the mock and is `#[serial]` per the harness rules in
//! `tests/common/mod.rs`.
//!
//! The CLI sleeps `interval` seconds between polls; tests collapse
//! that to a fixed millisecond delay with `ALS_LOGIN_POLL_MS=<ms>` so
//! wall-clock stays under ~1 s per case.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{SERVER, cli_with_home};
use serde_json::Value;
use serial_test::serial;

/// The CLI polls at this cadence regardless of the server-provided
/// `interval` for the test runs (the override is honored only when
/// set; production is unaffected).
const TEST_POLL_MS: &str = "60";

/// Convert the `assert_cmd::Command` built by `cli_with_home` into a
/// matching `std::process::Command` so we can `spawn()` it and
/// interact with the child mid-execution.
fn build_spawnable(home: &Path) -> Command {
    let bin = assert_cmd::cargo::cargo_bin("als");
    let mut cmd = Command::new(bin);
    cmd.env("HOME", home);
    cmd.env("XDG_CONFIG_HOME", home.join(".config"));
    cmd.env("ALS_API", SERVER.url.as_str());
    // The CLI's login path falls back to the on-disk config when there
    // is no token yet, so deliberately do NOT set ALS_TOKEN here.
    cmd.env("ALS_LOGIN_POLL_MS", TEST_POLL_MS);
    cmd
}

fn write_api_config(home: &Path) {
    let cfg_path = home.join(".config/als/config.toml");
    std::fs::write(
        &cfg_path,
        format!("[default]\napi = \"{}\"\n", SERVER.url.as_str()),
    )
    .expect("write config.toml");
}

fn read_token_from_config(home: &Path) -> Option<String> {
    let cfg_path = home.join(".config/als/config.toml");
    let raw = std::fs::read_to_string(&cfg_path).ok()?;
    // Cheap targeted parse: locate the `token = "..."` line under the
    // `[default]` table. Avoids pulling toml::Value (whose entry point
    // moved in toml v1) and keeps the helper trivially correct.
    let mut in_default = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_default = trimmed == "[default]";
            continue;
        }
        if !in_default {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("token") {
            let rest = rest.trim_start();
            let rest = rest.strip_prefix('=')?.trim();
            let rest = rest.strip_prefix('"')?;
            let end = rest.find('"')?;
            return Some(rest[..end].to_owned());
        }
    }
    None
}

/// Poll the mock-server state until a device grant exists, then return
/// its `userCode`. Bails out after `wait` elapses.
fn wait_for_user_code(wait: Duration) -> String {
    let deadline = Instant::now() + wait;
    loop {
        let snap = SERVER.snapshot();
        if let Some(code) = snap
            .get("deviceGrants")
            .and_then(Value::as_array)
            .and_then(|g| g.first())
            .and_then(|g| g.get("userCode"))
            .and_then(Value::as_str)
        {
            return code.to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "no device grant materialised within {wait:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[serial]
fn login_happy_path_persists_token() {
    SERVER.reset();
    let (_, home_path): (assert_cmd::Command, PathBuf) = cli_with_home();
    write_api_config(&home_path);

    let mut child = build_spawnable(&home_path)
        .args(["auth", "login", "--no-qr"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn als");

    let user_code = wait_for_user_code(Duration::from_secs(5));
    SERVER.device_action(&user_code, "approve");

    let status = child.wait().expect("wait als");
    assert!(status.success(), "login should succeed, got {status:?}");

    let token = read_token_from_config(&home_path).expect("token persisted");
    assert!(token.starts_with("tk_"), "unexpected token: {token}");
}

#[test]
#[serial]
fn login_handles_pending_then_authorized() {
    SERVER.reset();
    let (_, home_path) = cli_with_home();
    write_api_config(&home_path);

    let mut child = build_spawnable(&home_path)
        .args(["auth", "login", "--no-qr"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn als");

    // Let several pending polls flow before approving.
    let user_code = wait_for_user_code(Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(220));
    SERVER.device_action(&user_code, "approve");

    let status = child.wait().expect("wait als");
    assert!(status.success(), "login should succeed, got {status:?}");
    assert!(
        read_token_from_config(&home_path).is_some(),
        "token should be persisted after pending->approved"
    );
}

#[test]
#[serial]
fn login_handles_slow_down_then_authorized() {
    SERVER.reset();
    let (_, home_path) = cli_with_home();
    write_api_config(&home_path);

    let mut child = build_spawnable(&home_path)
        .args(["auth", "login", "--no-qr"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn als");

    let user_code = wait_for_user_code(Duration::from_secs(5));
    // First flip to slow_down_once. The next poll returns slow_down,
    // and the mock then resets the grant to `pending`.
    SERVER.device_action(&user_code, "slow_down_once");
    // Give the CLI time to observe the slow_down response.
    std::thread::sleep(Duration::from_millis(200));
    // Confirm the slow_down was consumed (status flipped back to pending).
    let snap = SERVER.snapshot();
    let grant = snap
        .get("deviceGrants")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
        .expect("device grant");
    assert_eq!(
        grant.get("status").and_then(Value::as_str),
        Some("pending"),
        "slow_down should have been consumed back to pending"
    );
    // Now grant access.
    SERVER.device_action(&user_code, "approve");

    let status = child.wait().expect("wait als");
    assert!(status.success(), "login should succeed, got {status:?}");
    assert!(read_token_from_config(&home_path).is_some());
}

#[test]
#[serial]
fn login_reports_access_denied() {
    SERVER.reset();
    let (_, home_path) = cli_with_home();
    write_api_config(&home_path);

    let mut child = build_spawnable(&home_path)
        .args(["auth", "login", "--no-qr"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn als");

    let user_code = wait_for_user_code(Duration::from_secs(5));
    SERVER.device_action(&user_code, "deny");

    let mut stderr_buf = String::new();
    child
        .stderr
        .as_mut()
        .expect("piped stderr")
        .read_to_string(&mut stderr_buf)
        .expect("read stderr");
    let status = child.wait().expect("wait als");
    assert!(!status.success(), "login should fail on access_denied");
    assert_eq!(status.code(), Some(1), "exit code should be 1");
    assert!(
        stderr_buf.contains("login_denied"),
        "stderr should mention login_denied:\n{stderr_buf}"
    );
    assert!(
        read_token_from_config(&home_path).is_none(),
        "no token should be persisted after denial"
    );
}

#[test]
#[serial]
fn login_reports_expired_token() {
    SERVER.reset();
    let (_, home_path) = cli_with_home();
    write_api_config(&home_path);

    let mut child = build_spawnable(&home_path)
        .args(["auth", "login", "--no-qr"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn als");

    let user_code = wait_for_user_code(Duration::from_secs(5));
    SERVER.device_action(&user_code, "expire");

    let mut stderr_buf = String::new();
    child
        .stderr
        .as_mut()
        .expect("piped stderr")
        .read_to_string(&mut stderr_buf)
        .expect("read stderr");
    let status = child.wait().expect("wait als");
    assert!(!status.success(), "login should fail on expired_token");
    assert_eq!(status.code(), Some(1), "exit code should be 1");
    assert!(
        stderr_buf.contains("login_timeout"),
        "stderr should mention login_timeout:\n{stderr_buf}"
    );
}

#[test]
#[serial]
fn login_prints_verification_url_for_manual_open() {
    // The CLI never spawns a browser — the operator copies the
    // verification URL from stdout. Confirm the URL actually appears
    // on stdout so that contract has direct coverage.
    SERVER.reset();
    let (_, home_path) = cli_with_home();
    write_api_config(&home_path);

    let mut child = build_spawnable(&home_path)
        .args(["auth", "login", "--no-qr"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn als");

    let user_code = wait_for_user_code(Duration::from_secs(5));
    SERVER.device_action(&user_code, "approve");

    let mut stdout = String::new();
    child
        .stdout
        .as_mut()
        .expect("piped stdout")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    let status = child.wait().expect("wait als");
    assert!(status.success(), "expected exit 0, got {status:?}");
    // The verification URL should appear so the user can copy it.
    assert!(
        stdout.contains("https://a.ls/verify"),
        "stdout should expose the verification URL: {stdout}"
    );
}

#[test]
#[serial]
fn login_no_qr_omits_block_glyphs() {
    SERVER.reset();
    let (_, home_path) = cli_with_home();
    write_api_config(&home_path);

    let mut child = build_spawnable(&home_path)
        .args(["auth", "login", "--no-qr"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn als");

    let user_code = wait_for_user_code(Duration::from_secs(5));
    SERVER.device_action(&user_code, "approve");

    let mut stdout = String::new();
    child
        .stdout
        .as_mut()
        .expect("piped stdout")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    let status = child.wait().expect("wait als");
    assert!(status.success());
    assert!(
        !stdout.contains('\u{2588}')
            && !stdout.contains('\u{2580}')
            && !stdout.contains('\u{2584}'),
        "--no-qr must suppress block glyphs: {stdout:?}"
    );
}

#[test]
#[serial]
fn login_with_paste_token_short_circuits() {
    SERVER.reset();
    let (_, home_path) = cli_with_home();
    write_api_config(&home_path);

    let assert = build_spawnable(&home_path)
        .args(["auth", "login", "-T", "tk_pasted_token_xyz"])
        .output()
        .expect("run als");
    assert!(
        assert.status.success(),
        "paste-token login should succeed: {assert:?}"
    );
    let token = read_token_from_config(&home_path).expect("token persisted");
    assert_eq!(token, "tk_pasted_token_xyz");
    // No device grant should have been created.
    let snap = SERVER.snapshot();
    let grants = snap.get("deviceGrants").and_then(Value::as_array);
    assert!(
        grants.is_none_or(<Vec<_>>::is_empty),
        "paste-token path must not touch /authorize"
    );
}
