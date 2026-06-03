//! Integration-test harness for `als-api`.
//!
//! Spawns the Bun + Hono mock als-api server (`tests/mock-server/src/main.ts`)
//! once per test binary and exposes helpers to drive control endpoints
//! (`/__test/reset`, `/__test/seed`, `/__test/inject`). This harness omits
//! the CLI-process plumbing — `als-api` tests exercise the typed client
//! directly.
//!
//! ## Discipline
//!
//! State on the mock server is process-global; the shared [`SERVER`] singleton
//! is intentionally `Lazy<Arc<MockServer>>` and the same process backs every
//! test in this binary. **Every test that touches [`SERVER`] MUST be marked
//! with `#[serial_test::serial]`.**
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
    process::{Child, ChildStdin, Command, Stdio},
    sync::Arc,
};

use once_cell::sync::Lazy;
use serde::Serialize;

/// Handle to a running Bun mock server. See the twin docstring in
/// `crates/als/tests/common/mod.rs` for the stdin-EOF cleanup design.
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
