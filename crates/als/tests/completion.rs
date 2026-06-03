//! E2E tests for `als completion <shell>` — verifies the compiled binary
//! emits a syntactically plausible completion script for every accepted
//! shell, and that an unknown shell value bails out via clap's
//! parse-error path (exit code 2 per `docs/cli-spec.md`).
//!
//! The completion command is local-only (no network, no config touch),
//! so each test uses `cli_with_home()` to skip mock-server bring-up and
//! still inherits the isolated `HOME` from the shared harness. Cases are
//! independent (no shared mutable state), so no `#[serial]` is needed.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use common::cli_with_home;
use predicates::prelude::*;

#[test]
fn completion_bash_non_empty() {
    let (mut cmd, _home) = cli_with_home();
    let output = cmd
        .args(["completion", "bash"])
        .output()
        .expect("spawn als");
    assert!(
        output.status.success(),
        "exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.stdout.len() > 100,
        "bash completion script should be > 100 bytes, got {}",
        output.stdout.len(),
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    assert!(
        stdout.contains("complete -F") || stdout.contains("_toss"),
        "bash script should declare a completion fn or `complete -F`:\n{stdout}",
    );
}

#[test]
fn completion_zsh_non_empty() {
    let (mut cmd, _home) = cli_with_home();
    cmd.args(["completion", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("#compdef als"));
}

#[test]
fn completion_fish_non_empty() {
    let (mut cmd, _home) = cli_with_home();
    cmd.args(["completion", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete -c als"));
}

#[test]
fn completion_powershell_non_empty() {
    let (mut cmd, _home) = cli_with_home();
    let output = cmd
        .args(["completion", "powershell"])
        .output()
        .expect("spawn als");
    assert!(
        output.status.success(),
        "exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        !output.stdout.is_empty(),
        "powershell completion stdout must not be empty",
    );
}

#[test]
fn completion_unknown_shell_exit_2() {
    let (mut cmd, _home) = cli_with_home();
    cmd.args(["completion", "ksh"]).assert().failure().code(2);
}
