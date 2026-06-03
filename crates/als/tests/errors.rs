//! Error-to-exit-code matrix. Each test injects a single failure via
//! `__test/inject` and confirms the CLI exits with the spec-mandated code.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

mod common;

use common::{SERVER, cli};
use predicates::prelude::*;
use serial_test::serial;

#[test]
#[serial]
fn unauthorized_exits_3() {
    SERVER.reset();
    SERVER.inject_failure("GET /api/sites", 401, "UNAUTHORIZED");
    cli()
        .arg("list")
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("auth_required"));
}

#[test]
#[serial]
fn quota_exceeded_exits_6() {
    SERVER.reset();
    SERVER.inject_failure("POST /api/deploy", 402, "QUOTA_EXCEEDED");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"<title>x</title>").unwrap();
    cli()
        .arg(dir.path())
        .assert()
        .failure()
        .code(6)
        .stderr(predicate::str::contains("quota_exceeded"));
}

#[test]
#[serial]
fn rate_limited_exits_7_and_surfaces_retry_after() {
    SERVER.reset();
    SERVER.inject_failure("GET /api/sites", 429, "RATE_LIMITED");
    cli()
        .arg("list")
        .assert()
        .failure()
        .code(7)
        .stderr(predicate::str::contains("rate_limited"))
        .stderr(predicate::str::contains("30"));
}

#[test]
#[serial]
fn server_error_exits_5() {
    SERVER.reset();
    SERVER.inject_failure("GET /api/sites", 500, "INTERNAL_ERROR");
    cli()
        .arg("list")
        .assert()
        .failure()
        .code(5)
        .stderr(predicate::str::contains("server_error"));
}

#[test]
#[serial]
fn not_found_on_site_lookup_exits_1() {
    // `als site <id>` against a 10-char id that the server doesn't
    // know triggers `site_ident::resolve`: the GET-by-id returns
    // 404, the resolver falls through to a name scan, and that too
    // finds nothing — so the user sees a single clean error that
    // calls out the failing input. The exact wording lives in
    // site_ident's "ambiguous/missing" branch; what we lock in here
    // is exit-code 1 plus the input string in the message (catches
    // silent-failure regressions like the activate `--version` one).
    SERVER.reset();
    cli()
        .args(["site", "zzzzzzzzzz"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("zzzzzzzzzz"))
        .stderr(predicate::str::contains("no site").or(predicate::str::contains("not_found")));
}

#[test]
#[serial]
fn archive_too_large_exits_1_with_documented_kind_and_hint() {
    // Per `cli-spec.md`, oversize uploads must surface the
    // `archive_too_large` kind plus the `als.toml exclude` hint so
    // users have a clear lever to pull.
    SERVER.reset();
    SERVER.inject_failure("POST /api/deploy", 413, "ARCHIVE_TOO_LARGE");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"<title>x</title>").unwrap();
    cli()
        .arg(dir.path())
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("archive_too_large"))
        .stderr(predicate::str::contains("als.toml"));
}

#[test]
#[serial]
fn archive_path_traversal_exits_1() {
    // 400 ARCHIVE_PATH_TRAVERSAL on deploy → exit 1, surface the
    // documented `path_traversal` kind so users see why their zip
    // was rejected.
    SERVER.reset();
    SERVER.inject_failure("POST /api/deploy", 400, "ARCHIVE_PATH_TRAVERSAL");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"<title>x</title>").unwrap();
    cli().arg(dir.path()).assert().failure().code(1).stderr(
        predicate::str::contains("path_traversal").or(predicate::str::contains("traversal")),
    );
}

#[test]
#[serial]
fn forbidden_response_exits_1_with_forbidden_kind() {
    // `FORBIDDEN` / `ACCOUNT_DISABLED` (e.g. admin-only resource for
    // a regular user, or a suspended account) maps to exit 1 — not 3
    // (3 is reserved for "your token is invalid or revoked"). The
    // kind surfaced in the diagnostic must match the wire code.
    SERVER.reset();
    SERVER.inject_failure("GET /api/sites", 403, "FORBIDDEN");
    cli()
        .arg("list")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("forbidden"));
}

#[test]
#[serial]
fn validation_error_on_deploy_exits_1_with_server_message() {
    // 400 VALIDATION_ERROR carries the server's message verbatim;
    // the CLI surfaces it under a generic `error:` line. Useful for
    // users to see exactly which field the server rejected.
    SERVER.reset();
    SERVER.inject_failure("POST /api/deploy", 400, "VALIDATION_ERROR");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), b"<title>x</title>").unwrap();
    cli()
        .arg(dir.path())
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("error"));
}
