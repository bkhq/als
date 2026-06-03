//! E2E coverage for `als auth status` / `als auth revoke`.
//!
//! `als auth login` lives in `login.rs`; these tests cover the two
//! other endpoints the `auth` subcommand depends on:
//!   * `auth status` → `GET /api/account/me` + `GET /api/quota/me`
//!   * `auth revoke` → `DELETE /api/tokens/:prefix`
//!
//! Both tests drive the binary against the shared Bun mock and assert
//! on both the user-facing output and the server-side state change
//! that should have happened as a side effect.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

mod common;

use std::fs;

use common::{SERVER, cli};
use serde_json::Value;
use serial_test::serial;

/// First 8 chars of the seeded default bearer token. Matches what the
/// CLI uses as the `prefix` argument to `DELETE /api/tokens/:prefix`.
const DEFAULT_TOKEN_FULL: &str = "tk_test0001abcdef";
const DEFAULT_TOKEN_PREFIX: &str = "tk_test0";

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

fn token_full_present(snap: &Value, full: &str) -> bool {
    snap["tokens"]
        .as_array()
        .is_some_and(|arr| arr.iter().any(|t| t["full"].as_str() == Some(full)))
}

fn stdout_of(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout")
}

#[test]
#[serial]
fn auth_status_renders_account_and_quota_from_wire() {
    // `als auth status` joins `/me` + `/quota/me` and renders both.
    // The mock seeds Alice with a known quota; assert the rendered
    // output surfaces the wire fields we expect users to read.
    SERVER.reset();
    let assert = cli().args(["auth", "status"]).assert().success();
    let stdout = stdout_of(&assert);

    // From `/api/account/me` (Alice, the default seeded user).
    assert!(
        stdout.contains("Alice Liu") && stdout.contains("alice@example.com"),
        "expected account name + email in status output:\n{stdout}"
    );
    // From `/api/quota/me` — Alice has 0 sites against the default
    // 100-site cap (see tests/mock-server/src/fixtures.ts).
    assert!(
        stdout.contains("0 / 100 sites"),
        "expected '0 / 100 sites' quota footer:\n{stdout}"
    );
    // Token rendering must mask, not echo the full bearer.
    assert!(
        stdout.contains(DEFAULT_TOKEN_PREFIX) && !stdout.contains(DEFAULT_TOKEN_FULL),
        "token should be prefix-masked, never full-printed:\n{stdout}"
    );
}

#[test]
#[serial]
fn expired_token_on_status_call_exits_3_with_login_hint() {
    // Boundary case the user hits when their bearer was rotated /
    // revoked on the server but still lives in their local config.
    // The server answers `/api/account/me` (the first call `auth
    // status` makes) with 401 UNAUTHORIZED, which the CLI maps to
    // `Error::Auth` → exit code 3 + `auth_required` line + the
    // "run `als login` again" hint per `cli-spec.md`.
    SERVER.reset();
    SERVER.inject_failure("GET /api/account/me", 401, "UNAUTHORIZED");
    let assert = cli().args(["auth", "status"]).assert().code(3);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("auth_required"),
        "expected `auth_required` diagnostic; got:\n{stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("login"),
        "expected the relogin hint in the diagnostic; got:\n{stderr}"
    );
}

#[test]
#[serial]
fn auth_revoke_deletes_token_server_side_and_clears_locally() {
    // Three coupled invariants must hold after `als auth revoke -y`:
    //   1. The server's `DELETE /api/tokens/:prefix` was honoured —
    //      the full token is gone from `state.tokens`.
    //   2. The local `config.toml` no longer carries the bearer, so
    //      subsequent calls re-prompt for login.
    //   3. The follow-up authenticated call exits with code 3
    //      (`Error::Auth`), matching `cli-spec.md`'s exit-code matrix.
    SERVER.reset();

    // Sanity: the seeded token is present pre-revoke.
    let pre = fetch_state();
    assert!(
        token_full_present(&pre, DEFAULT_TOKEN_FULL),
        "default token should be seeded before the test runs: {pre}"
    );

    // The shared `cli()` helper writes the token into the isolated
    // HOME's `config.toml` via the `ALS_TOKEN` env override; the
    // revoke command first reads it, then issues the DELETE, then
    // rewrites the file with an empty token.
    let (mut cmd, home) = common::cli_with_home();
    cmd.env("ALS_API", SERVER.url.as_str());
    cmd.env("ALS_TOKEN", DEFAULT_TOKEN_FULL);
    cmd.args(["auth", "revoke", "-y"]).assert().success();

    // Server side: token gone.
    let post = fetch_state();
    assert!(
        !token_full_present(&post, DEFAULT_TOKEN_FULL),
        "server must have deleted the token after revoke: {post}"
    );

    // Local side: token cleared from disk. Revoke writes the config
    // back via `write_token`, so re-reading the file should show an
    // empty `token = ""` entry.
    let config = fs::read_to_string(home.join(".config/als/config.toml"))
        .expect("config.toml exists after revoke");
    assert!(
        config.contains("token = \"\"") || !config.contains(DEFAULT_TOKEN_FULL),
        "config.toml should no longer carry the revoked token; body:\n{config}"
    );

    // Follow-up authenticated call: without a server-side token and
    // without a local one, the next call exits 1 with a `config`
    // error — the binary bails out at the local config layer before
    // ever issuing a request. (`Error::Auth` exit 3 covers the
    // server-says-401 case, which is a different code path.)
    // `ALS_TOKEN` is explicitly cleared via env_remove so the
    // override doesn't mask the cleared disk state.
    let mut next = assert_cmd::Command::cargo_bin("als").expect("locate als binary");
    next.env("HOME", &home);
    next.env("XDG_CONFIG_HOME", home.join(".config"));
    next.env("ALS_API", SERVER.url.as_str());
    next.env_remove("ALS_TOKEN");
    let assert = next.args(["auth", "status"]).assert().code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.to_lowercase().contains("token"),
        "expected missing-token message after revoke; got:\n{stderr}"
    );
}
