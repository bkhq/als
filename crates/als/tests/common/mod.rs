//! Integration-test harness.
//!
//! Spawns the Bun + Hono mock als-api server (`tests/mock-server/src/main.ts`)
//! once per test binary, parses the `LISTENING <url>` line from stdout, and
//! exposes helpers to drive control endpoints (`/__test/reset`, `/__test/seed`,
//! `/__test/inject`) and to build pre-configured `assert_cmd::Command`
//! invocations of the `als` binary.
//!
//! ## Discipline
//!
//! State on the mock server is process-global; the shared [`SERVER`] singleton
//! is intentionally `Lazy<Arc<MockServer>>` and the same process backs every
//! test in this binary. **Every test that touches [`SERVER`] MUST be marked
//! with `#[serial_test::serial]`.** The `e2e` nextest profile additionally
//! sets `test-threads = 1` as a belt-and-suspenders.
//!
//! ## Bun entry point
//!
//! [`MockServer::spawn`] reads the entry from the `ALS_TEST_MOCK_SERVER`
//! environment variable when set (CI uses this), otherwise falls back to the
//! in-tree `tests/mock-server/src/main.ts` resolved against
//! `CARGO_MANIFEST_DIR`.

// Test-harness helpers (not test functions themselves) trip several lints
// despite `allow-*-in-tests = true` in `clippy.toml`; `allow-*-in-tests`
// targets test items only. `non_std_lazy_statics` is intentionally allowed:
// the workspace pins `once_cell` for these harnesses.
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::non_std_lazy_statics)]
#![allow(dead_code)]
// `tests/common/mod.rs` is included via `mod common;` from each integration
// test root; the `pub` API is reachable from the test crate but not from any
// external consumer, so silence the workspace `unreachable_pub` warning here.
#![allow(unreachable_pub)]

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
};

use once_cell::sync::Lazy;
use serde::Serialize;
use tempfile::TempDir;

/// Handle to a running Bun mock server.
///
/// The shared `Lazy<Arc<MockServer>>` singleton (see [`SERVER`]) outlives the
/// test functions, so `Drop` runs only in the rare manual-drop path. The
/// orphan-on-exit problem is instead solved on the bun side: `main.ts`
/// watches its stdin and exits on EOF. We pipe stdin from this struct and
/// hold the [`ChildStdin`] for the lifetime of `MockServer`. When the test
/// process exits — graceful, panic, or SIGKILL — the kernel closes our pipe;
/// bun reads EOF and exits cleanly. The workspace `unsafe_code = "forbid"`
/// lint rules out `#[ctor::dtor]` / `libc::atexit` / `prctl(PR_SET_PDEATHSIG)`
/// belts-and-suspenders, so the EOF channel is the sole cleanup mechanism.
pub struct MockServer {
    child: Child,
    /// Write end of bun's stdin kept open for the lifetime of `MockServer`.
    /// When the kernel closes our fds at process exit, bun reads EOF and
    /// exits — even under SIGKILL where no signal can propagate.
    _stdin: ChildStdin,
    /// Base URL the server is listening on (e.g. `http://127.0.0.1:54321`).
    pub url: url::Url,
    http: reqwest::blocking::Client,
}

impl MockServer {
    /// Spawn `bun run <entry> --port 0` and block until the server prints
    /// `LISTENING <url>` on stdout.
    pub fn spawn() -> Self {
        let entry = std::env::var("ALS_TEST_MOCK_SERVER").unwrap_or_else(|_| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/mock-server/src/main.ts",
            )
            .to_string()
        });
        let mut child = Command::new("bun")
            .args(["run", &entry, "--port", "0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn bun mock server (is `bun` on PATH?)");
        let stdin = child.stdin.take().expect("child stdin piped");
        let stdout = child.stdout.take().expect("child stdout piped");
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .expect("read LISTENING line from mock server stdout");
        assert!(
            bytes != 0,
            "mock server stdout closed before printing `LISTENING <url>`",
        );
        let url_str = line
            .strip_prefix("LISTENING ")
            .unwrap_or_else(|| panic!("expected `LISTENING <url>` line, got: {line:?}"))
            .trim();
        let url: url::Url = url_str.parse().expect("parse mock server URL");
        Self {
            child,
            _stdin: stdin,
            url,
            http: reqwest::blocking::Client::new(),
        }
    }

    /// POST `/__test/reset` — clear all server-side state.
    pub fn reset(&self) {
        let endpoint = self.url.join("/__test/reset").expect("join reset endpoint");
        let resp = self.http.post(endpoint).send();
        assert!(resp.is_ok(), "mock reset failed: {resp:?}");
    }

    /// POST `/__test/seed` — seed entities (sites/users/tokens/etc).
    pub fn seed(&self, fixture: impl Serialize) {
        let endpoint = self.url.join("/__test/seed").expect("join seed endpoint");
        let resp = self.http.post(endpoint).json(&fixture).send();
        assert!(resp.is_ok(), "mock seed failed: {resp:?}");
    }

    /// POST `/__test/inject` — make the next request to `path` return the
    /// given status / envelope code.
    pub fn inject_failure(&self, path: &str, status: u16, code: &str) {
        let endpoint = self
            .url
            .join("/__test/inject")
            .expect("join inject endpoint");
        let body = serde_json::json!({ "path": path, "status": status, "code": code });
        let resp = self.http.post(endpoint).json(&body).send();
        assert!(resp.is_ok(), "mock inject failed: {resp:?}");
    }

    /// GET `/__test/state` — fetch the JSON snapshot of all server-side
    /// state (sites, tokens, deviceGrants, …).
    pub fn snapshot(&self) -> serde_json::Value {
        let endpoint = self.url.join("/__test/state").expect("join state");
        let body: serde_json::Value = self
            .http
            .get(endpoint)
            .send()
            .expect("GET state")
            .json()
            .expect("parse state json");
        body.get("data").cloned().unwrap_or(body)
    }

    /// POST `/__test/device/approve` — drive a pending device-flow
    /// grant through the test harness. `action` is one of `approve`,
    /// `deny`, `slow_down_once`, or `expire`.
    pub fn device_action(&self, user_code: &str, action: &str) {
        let endpoint = self
            .url
            .join("/__test/device/approve")
            .expect("join device approve");
        let body = serde_json::json!({ "user_code": user_code, "action": action });
        let resp = self.http.post(endpoint).json(&body).send();
        assert!(resp.is_ok(), "device approve failed: {resp:?}");
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Single mock-server instance shared across all tests in this binary.
///
/// Tests using `SERVER` MUST be marked `#[serial_test::serial]`.
pub static SERVER: Lazy<Arc<MockServer>> = Lazy::new(|| Arc::new(MockServer::spawn()));

/// Per-process registry that keeps each isolated `HOME` `TempDir` alive for
/// the lifetime of the test binary. Without this the directory would be
/// deleted as soon as [`cli`] returned, racing the child process.
static TEMP_HOMES: Lazy<Mutex<Vec<TempDir>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Build an `assert_cmd::Command` for the `als` binary preset with
/// `ALS_API`, a deterministic `ALS_TOKEN`, and an isolated `HOME` pointing
/// at a fresh tempdir.
///
/// `als_core::config::load` still opens the on-disk config before applying
/// env-var overrides, so an empty `config.toml` is staged at every plausible
/// XDG / Apple location under the isolated `HOME`. `ALS_API` / `ALS_TOKEN`
/// then supply the values for the active profile.
pub fn cli() -> assert_cmd::Command {
    let (mut cmd, _home) = cli_with_home();
    cmd.env("ALS_API", SERVER.url.as_str());
    cmd.env("ALS_TOKEN", "tk_test0001abcdef");
    cmd
}

/// Like [`cli`] but does **not** set `ALS_API` / `ALS_TOKEN` envs and
/// returns the isolated `HOME` path so the caller can seed an on-disk
/// `config.toml` before invoking the binary. Used by E2E suites that
/// exercise local-only commands (`config`, `completion`).
pub fn cli_with_home() -> (assert_cmd::Command, PathBuf) {
    let home: TempDir = tempfile::tempdir().expect("create isolated tmp HOME");
    let home_path: PathBuf = home.path().to_path_buf();
    stage_empty_config(&home_path);
    TEMP_HOMES
        .lock()
        .expect("temp-home registry lock")
        .push(home);

    let mut cmd = assert_cmd::Command::cargo_bin("als").expect("locate als binary");
    cmd.env("HOME", &home_path);
    cmd.env("XDG_CONFIG_HOME", home_path.join(".config"));
    (cmd, home_path)
}

fn stage_empty_config(home: &std::path::Path) {
    // Linux XDG and Apple Application-Support paths both anchor on `$HOME`;
    // stage an empty file under each so `etcetera::choose_base_strategy()`
    // finds it regardless of the test runner platform.
    for rel in [
        ".config/als/config.toml",
        "Library/Application Support/als/config.toml",
    ] {
        let path = home.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create isolated config dir");
        }
        std::fs::write(&path, "").expect("write empty config.toml");
    }
}
