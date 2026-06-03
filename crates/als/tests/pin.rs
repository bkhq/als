//! E2E coverage for the project-pinning feature
//! (`docs/cli-spec.md#project-pinning`):
//!
//! * Re-deploying the same path picks up the local pin and pushes a
//!   new version to the existing site (URL stable across calls).
//! * `--name` works as the explicit identity key across paths.
//! * `--new` opts out of both forms of dedup.
//! * `als unpin` removes the binding so the next deploy starts fresh.
//! * `als rm` clears the local pin pointing at the deleted site.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::SERVER;
use serde_json::Value;
use serial_test::serial;
use tempfile::TempDir;

/// Build an `als` command pre-wired to the mock server, anchored on
/// the given persistent HOME so two consecutive invocations share
/// the same `~/.config/als/sites/` directory.
fn cli_for_home(home: &Path) -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::cargo_bin("als").expect("locate als binary");
    cmd.env("HOME", home);
    cmd.env("XDG_CONFIG_HOME", home.join(".config"));
    cmd.env("ALS_API", SERVER.url.as_str());
    cmd.env("ALS_TOKEN", "tk_test0001abcdef");
    cmd
}

/// Stage an empty config so `als_core::config::load` succeeds before
/// the env overrides kick in.
fn fresh_persistent_home() -> TempDir {
    let home = tempfile::tempdir().expect("create isolated HOME");
    for rel in [
        ".config/als/config.toml",
        "Library/Application Support/als/config.toml",
    ] {
        let path = home.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir parent");
        }
        std::fs::write(&path, "").expect("seed empty config");
    }
    home
}

fn dir_with_one_file() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>fixture</title>",
    )
    .expect("write index.html");
    dir
}

fn site_count() -> usize {
    let url = SERVER.url.join("/__test/state").expect("join state");
    let body: Value = reqwest::blocking::Client::new()
        .get(url)
        .send()
        .expect("GET state")
        .json()
        .expect("parse state json");
    body.get("data")
        .and_then(|d| d.get("sites"))
        .and_then(|s| s.as_array())
        .map_or(0, Vec::len)
}

/// Path to the per-site pin store directory under the given HOME.
fn sites_dir(home: &Path) -> PathBuf {
    home.join(".config/als/sites")
}

/// Number of `*.toml` files (i.e. pin entries) under `sites_dir(home)`.
fn pin_count(home: &Path) -> usize {
    let dir = sites_dir(home);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.path().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("toml")
        })
        .count()
}

/// Concatenate every pin file's contents. Test helper for substring
/// assertions ("the projects store no longer mentions <id>").
fn read_all_pins(home: &Path) -> String {
    let dir = sites_dir(home);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return String::new();
    };
    let mut buf = String::new();
    for entry in entries.filter_map(Result::ok) {
        let p = entry.path();
        if p.extension().and_then(|x| x.to_str()) == Some("toml")
            && let Ok(s) = std::fs::read_to_string(&p)
        {
            buf.push_str(&s);
            buf.push('\n');
        }
    }
    buf
}

fn stdout_of(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("stdout utf8")
}

#[test]
#[serial]
fn second_deploy_of_same_path_keeps_the_same_url() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let first = cli_for_home(&home_path).arg(dir.path()).assert().success();
    let first_url = extract_url(&stdout_of(&first)).expect("first URL");
    assert_eq!(site_count(), 1, "first deploy creates exactly one site");

    let second = cli_for_home(&home_path).arg(dir.path()).assert().success();
    let second_url = extract_url(&stdout_of(&second)).expect("second URL");

    assert_eq!(
        first_url, second_url,
        "URL must be stable across re-deploys: first={first_url:?} second={second_url:?}"
    );
    assert_eq!(
        site_count(),
        1,
        "second deploy should add a version server-side, not a new site"
    );

    // The pin store records exactly one binding for the canonical path.
    assert_eq!(
        pin_count(&home_path),
        1,
        "expected one pin file under sites/, found:\n{}",
        read_all_pins(&home_path),
    );
}

#[test]
#[serial]
fn explicit_name_pins_across_different_paths() {
    SERVER.reset();
    let dir_a = dir_with_one_file();
    let dir_b = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let a = cli_for_home(&home_path)
        .arg(dir_a.path())
        .arg("--name")
        .arg("sharedproj")
        .assert()
        .success();
    let url_a = extract_url(&stdout_of(&a)).expect("URL A");

    let b = cli_for_home(&home_path)
        .arg(dir_b.path())
        .arg("--name")
        .arg("sharedproj")
        .assert()
        .success();
    let url_b = extract_url(&stdout_of(&b)).expect("URL B");

    assert_eq!(url_a, url_b, "same --name must resolve to the same URL");
    assert_eq!(
        site_count(),
        1,
        "name-based dedup keeps one site server-side"
    );
}

#[test]
#[serial]
fn new_flag_forces_fresh_site_despite_prior_pin() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let first = cli_for_home(&home_path).arg(dir.path()).assert().success();
    let url_first = extract_url(&stdout_of(&first)).expect("URL");

    let second = cli_for_home(&home_path)
        .arg(dir.path())
        .arg("--new")
        .assert()
        .success();
    let url_second = extract_url(&stdout_of(&second)).expect("URL");

    assert_ne!(
        url_first, url_second,
        "--new must allocate a fresh URL, got {url_first:?} == {url_second:?}"
    );
    assert_eq!(site_count(), 2, "--new should add a separate site");
}

#[test]
#[serial]
fn unpin_clears_the_local_binding() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    cli_for_home(&home_path).arg(dir.path()).assert().success();
    assert_eq!(
        pin_count(&home_path),
        1,
        "expected one pin file after first deploy",
    );

    let canonical = dir.path().canonicalize().expect("canonicalize");
    cli_for_home(&home_path)
        .arg("unpin")
        .arg(canonical.to_string_lossy().as_ref())
        .assert()
        .success();

    assert_eq!(
        pin_count(&home_path),
        0,
        "pin store should be empty after unpin:\n{}",
        read_all_pins(&home_path),
    );

    // `unpin` only clears the local cache; the auto-written
    // `als.toml.id` is the cross-machine pin and still binds the
    // project to the existing site. To exercise the "starts fresh
    // after unpin" path we have to drop that file too — the same
    // thing a user would do to truly detach.
    let _ = fs::remove_file(dir.path().join("als.toml"));

    cli_for_home(&home_path).arg(dir.path()).assert().success();
    assert_eq!(
        site_count(),
        2,
        "unpin + remove als.toml + redeploy should add a new site"
    );
}

#[test]
#[serial]
fn stale_local_name_falls_back_to_id_anchor() {
    // Cross-machine rename scenario: machine A renames the site to a
    // new name, machine B still has the original name in its local
    // pin. Without the `site_id` anchor on the wire the deploy from B
    // would create a duplicate site under the stale name; with it,
    // the server resolves by id, ignores the stale name, and pushes
    // a version to the same site.
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let first = cli_for_home(&home_path)
        .arg(dir.path())
        .arg("--name")
        .arg("orig-name")
        .assert()
        .success();
    let first_url = extract_url(&stdout_of(&first)).expect("first URL");
    let site_id = site_id_from_url(&first_url).expect("id from URL");

    // Simulate machine A renaming the site server-side: edit the
    // server state directly through the test control plane. The local
    // pin on this machine still says `orig-name`.
    rename_site_server_side(&site_id, "new-name");

    // Second deploy from the same path. The local pin's `name` is
    // stale (`orig-name`), but its `id` is still correct. The wire
    // request must include `site_id` so the server resolves by id.
    let second = cli_for_home(&home_path).arg(dir.path()).assert().success();
    let second_url = extract_url(&stdout_of(&second)).expect("second URL");

    assert_eq!(
        first_url, second_url,
        "id anchor must keep the URL stable across a cross-machine rename"
    );
    assert_eq!(
        site_count(),
        1,
        "the server must not create a duplicate site under the stale name"
    );
}

/// Drive a server-side rename through the mock's test control plane.
/// Mirrors what `als site <id> --name <new>` would do on another
/// machine; we go through the control plane because the binary under
/// test would otherwise refresh the local pin file we're trying to
/// keep stale.
fn rename_site_server_side(id: &str, new_name: &str) {
    let url = SERVER
        .url
        .join(&format!("/__test/sites/{id}/rename"))
        .expect("join rename endpoint");
    let body = serde_json::json!({ "name": new_name });
    let resp = reqwest::blocking::Client::new()
        .post(url)
        .json(&body)
        .send()
        .expect("rename POST");
    assert!(
        resp.status().is_success(),
        "mock rename failed: {}",
        resp.status()
    );
}

#[test]
#[serial]
fn rename_via_deploy_keeps_id_and_url() {
    // `--name new` on a path with a prior pin means "rename this site",
    // not "fork to a new one". The deploy must still anchor by id, the
    // server must apply the rename, the URL must stay put, and the
    // local pin file must be rewritten to the new name.
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let first = cli_for_home(&home_path)
        .arg(dir.path())
        .arg("--name")
        .arg("before")
        .assert()
        .success();
    let first_url = extract_url(&stdout_of(&first)).expect("first URL");
    let site_id = site_id_from_url(&first_url).expect("id from URL");

    // Change content too — otherwise the no-op-skip would fire on the
    // identical-bytes branch before the rename can be observed.
    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>renamed</title>",
    )
    .expect("rewrite index");

    let second = cli_for_home(&home_path)
        .arg(dir.path())
        .arg("--name")
        .arg("after")
        .assert()
        .success();
    let second_url = extract_url(&stdout_of(&second)).expect("second URL");

    assert_eq!(
        first_url, second_url,
        "id anchor must keep the URL stable across a rename-via-deploy"
    );
    assert_eq!(site_count(), 1, "rename should not fork the site");

    // Server side: the projectName must reflect the new name.
    let url = SERVER.url.join("/__test/state").expect("join state");
    let body: Value = reqwest::blocking::Client::new()
        .get(url)
        .send()
        .expect("GET state")
        .json()
        .expect("parse state json");
    let server_name = body["data"]["sites"][0]["projectName"]
        .as_str()
        .expect("projectName");
    assert_eq!(server_name, "after");

    // Local pin store: only the renamed file should exist; the old
    // `<id>_before.toml` must be gone.
    let pins = read_all_pins(&home_path);
    assert!(
        pins.contains(&site_id) && pins.contains("after"),
        "renamed pin should mention id and new name:\n{pins}"
    );
    assert!(
        !pins.contains("before"),
        "stale pin under old name should have been removed:\n{pins}"
    );
}

#[test]
#[serial]
fn first_deploy_writes_id_back_to_als_toml() {
    // Auto-write behaviour: after a successful deploy from a directory
    // input, `als.toml` should contain the server-assigned `id`. The
    // file is created if it didn't exist; existing fields are kept.
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let first = cli_for_home(&home_path).arg(dir.path()).assert().success();
    let url = extract_url(&stdout_of(&first)).expect("URL");
    let id = site_id_from_url(&url).expect("id from URL");

    let toml_path = dir.path().join("als.toml");
    let text = fs::read_to_string(&toml_path).expect("als.toml written");
    assert!(
        text.contains(&format!("id = \"{id}\"")),
        "als.toml should pin the server id; body:\n{text}"
    );
}

#[test]
#[serial]
fn als_toml_id_pins_across_machines() {
    // Cross-machine pin via the committed `als.toml.id`. Machine A
    // deploys (auto-writes the id back into `als.toml`). Machine B
    // gets the same source tree (including the now-pinned id) but a
    // fresh pin store. The second deploy must hit the same site
    // because the wire `site_id` came from the file, not from the
    // local cache.
    SERVER.reset();
    let dir = dir_with_one_file();

    // Machine A.
    let home_a = fresh_persistent_home();
    let first = cli_for_home(home_a.path())
        .arg(dir.path())
        .assert()
        .success();
    let url_a = extract_url(&stdout_of(&first)).expect("URL A");

    // Sanity check: machine A's auto-write produced an als.toml with
    // the server-issued id.
    let toml_text =
        fs::read_to_string(dir.path().join("als.toml")).expect("als.toml written on machine A");
    assert!(toml_text.contains("id ="), "als.toml: {toml_text}");

    // Machine B: fresh HOME (no pin store), same source tree. The
    // committed `als.toml.id` is the only anchor available.
    let home_b = fresh_persistent_home();
    let second = cli_for_home(home_b.path())
        .arg(dir.path())
        .assert()
        .success();
    let url_b = extract_url(&stdout_of(&second)).expect("URL B");

    assert_eq!(
        url_a, url_b,
        "als.toml.id must give the same URL on a different machine"
    );
    assert_eq!(site_count(), 1, "single site server-side");
}

#[test]
#[serial]
fn als_toml_name_pins_across_machines() {
    SERVER.reset();
    let dir = dir_with_one_file();
    std::fs::write(dir.path().join("als.toml"), br#"name = "shared-name""#)
        .expect("write als.toml");

    // Machine A: first deploy reads als.toml.name, server creates site
    // under that name.
    let home_a = fresh_persistent_home();
    let first = cli_for_home(home_a.path())
        .arg(dir.path())
        .assert()
        .success();
    let url_a = extract_url(&stdout_of(&first)).expect("first URL");

    // Machine B (fresh HOME → empty pin store) re-runs from the same
    // source tree. The committed als.toml.name still resolves to the
    // existing site server-side, so the URL is stable.
    let home_b = fresh_persistent_home();
    let second = cli_for_home(home_b.path())
        .arg(dir.path())
        .assert()
        .success();
    let url_b = extract_url(&stdout_of(&second)).expect("second URL");

    assert_eq!(
        url_a, url_b,
        "als.toml.name must give the same URL on a different machine"
    );
    assert_eq!(site_count(), 1, "single site server-side");
}

#[test]
#[serial]
fn unchanged_redeploy_skips_the_network_call() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    cli_for_home(&home_path).arg(dir.path()).assert().success();
    assert_eq!(site_count(), 1);

    let second = cli_for_home(&home_path).arg(dir.path()).assert().success();
    let stdout = stdout_of(&second);
    assert!(
        stdout.contains("No changes since last deploy"),
        "second deploy with identical content should skip:\n{stdout}"
    );
    // Server side: the deploy never landed, so site count and version
    // history are both unchanged.
    assert_eq!(
        site_count(),
        1,
        "skip path must not create a new site / version on the server"
    );

    // A content change tips the digest and forces a real deploy again.
    std::fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>changed</title>",
    )
    .expect("rewrite index.html");
    let third = cli_for_home(&home_path).arg(dir.path()).assert().success();
    let stdout_third = stdout_of(&third);
    assert!(
        stdout_third.contains("Deployed:"),
        "content change must trigger a real deploy:\n{stdout_third}"
    );
}

#[test]
#[serial]
fn metadata_flag_overrides_no_change_skip() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    cli_for_home(&home_path).arg(dir.path()).assert().success();

    // Same content, but `--expires` is a metadata-touching flag, so
    // the deploy must run regardless of the matching content hash.
    let second = cli_for_home(&home_path)
        .arg(dir.path())
        .arg("--expires")
        .arg("30d")
        .assert()
        .success();
    let stdout = stdout_of(&second);
    assert!(
        stdout.contains("Deployed:") && !stdout.contains("No changes"),
        "any metadata flag should bypass the skip:\n{stdout}"
    );
}

#[test]
#[serial]
fn rm_drops_matching_local_pin() {
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let first = cli_for_home(&home_path).arg(dir.path()).assert().success();
    let url = extract_url(&stdout_of(&first)).expect("URL");
    let site_id = site_id_from_url(&url).expect("id from url");

    // -y bypasses the interactive prompt.
    cli_for_home(&home_path)
        .arg("rm")
        .arg(&site_id)
        .arg("-y")
        .assert()
        .success();

    let pins = read_all_pins(&home_path);
    assert!(
        !pins.contains(&site_id),
        "pin store should no longer mention {site_id}:\n{pins}"
    );
}

#[test]
#[serial]
fn two_machines_share_als_toml_id_and_push_distinct_versions() {
    // Both machines deploy through the same committed `als.toml.id`.
    // The server must keep a single site (id anchor wins on every
    // deploy) and accumulate one version per upload.
    SERVER.reset();
    let dir = dir_with_one_file();

    // Machine A: first deploy auto-writes the id back into als.toml.
    let home_a = fresh_persistent_home();
    let first = cli_for_home(home_a.path())
        .arg(dir.path())
        .assert()
        .success();
    let url_a = extract_url(&stdout_of(&first)).expect("URL A");
    let id_a = site_id_from_url(&url_a).expect("id A");

    // Touch the source so the no-op-skip on machine B doesn't bypass
    // the upload — we want B to actually hit the network.
    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>B</title>",
    )
    .expect("rewrite index.html");

    // Machine B: same source tree (including the committed
    // `als.toml.id`), fresh HOME (no local pin cache).
    let home_b = fresh_persistent_home();
    let second = cli_for_home(home_b.path())
        .arg(dir.path())
        .assert()
        .success();
    let url_b = extract_url(&stdout_of(&second)).expect("URL B");
    let id_b = site_id_from_url(&url_b).expect("id B");

    assert_eq!(id_a, id_b, "committed id must anchor both machines");

    // Server state: one site, two versions.
    let snap = fetch_state();
    let sites = snap["sites"].as_array().expect("sites array");
    assert_eq!(sites.len(), 1, "exactly one site; got {sites:?}");
    let versions = sites[0]["versions"].as_array().expect("versions array");
    assert_eq!(
        versions.len(),
        2,
        "expected 2 versions (one per machine); got {versions:?}"
    );
}

#[test]
#[serial]
fn stale_als_toml_id_after_server_rm_surfaces_clean_error() {
    // Machine A deploys + deletes the site server-side. Machine B's
    // committed `als.toml.id` is now pointing at a non-existent site;
    // its next deploy must fail loudly with `site_not_found` rather
    // than silently allocating a new one under the stale id.
    SERVER.reset();
    let dir = dir_with_one_file();

    let home_a = fresh_persistent_home();
    let first = cli_for_home(home_a.path())
        .arg(dir.path())
        .assert()
        .success();
    let url = extract_url(&stdout_of(&first)).expect("URL");
    let site_id = site_id_from_url(&url).expect("id");

    // Machine A removes the site (and clears its own pin entry).
    cli_for_home(home_a.path())
        .arg("rm")
        .arg(&site_id)
        .arg("-y")
        .assert()
        .success();

    // Machine B: fresh HOME, source tree still carries the committed
    // (now-dead) id. The deploy must fail.
    let home_b = fresh_persistent_home();
    let assert = cli_for_home(home_b.path())
        .arg(dir.path())
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("stderr utf8");
    assert!(
        stderr.to_lowercase().contains("not found")
            || stderr.to_lowercase().contains("site_not_found"),
        "expected site-not-found-style message, got:\n{stderr}"
    );

    // Most important invariant: no rogue allocation. Server still
    // holds nothing live; the stale id did not silently mint a new site.
    let snap = fetch_state();
    let active: Vec<_> = snap["sites"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter(|s| s["releasedAt"].is_null())
        .cloned()
        .collect();
    assert!(
        active.is_empty(),
        "no new site should have been allocated under the stale id; got: {active:?}"
    );
}

#[test]
#[serial]
fn server_side_rename_propagates_into_next_deploy() {
    // Cross-command coherence: rename via the dedicated `als site`
    // command (PATCH /api/sites/:id), then `als <path>` from the same
    // dir. The id-anchored re-deploy must see the new name in the
    // server response and update the local pin file accordingly. The
    // committed `als.toml.id` is unchanged (rename doesn't reissue ids).
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let first = cli_for_home(&home_path)
        .arg(dir.path())
        .arg("--name")
        .arg("before")
        .assert()
        .success();
    let url = extract_url(&stdout_of(&first)).expect("URL");
    let site_id = site_id_from_url(&url).expect("id");

    cli_for_home(&home_path)
        .args(["site", &site_id, "--name", "after"])
        .assert()
        .success();

    // Tip the digest so the no-op skip doesn't short-circuit the
    // second deploy; the rename only propagates when the wire request
    // actually happens.
    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>post-rename</title>",
    )
    .expect("rewrite index");

    cli_for_home(&home_path).arg(dir.path()).assert().success();

    // Server: one site, name == "after".
    let snap = fetch_state();
    assert_eq!(
        snap["sites"][0]["projectName"].as_str(),
        Some("after"),
        "server name not updated: {snap}"
    );

    // Local pin file: rewritten under the new name.
    let pins = read_all_pins(&home_path);
    assert!(
        pins.contains("after") && !pins.contains("before"),
        "pin store should carry the new name only; got:\n{pins}"
    );

    // als.toml.id: unchanged (rename does not reissue ids).
    let toml_text = fs::read_to_string(dir.path().join("als.toml")).expect("als.toml");
    assert!(
        toml_text.contains(&format!("id = \"{site_id}\"")),
        "als.toml id should be unchanged; body:\n{toml_text}"
    );
}

#[test]
#[serial]
fn deploy_keeps_server_pin_and_als_toml_in_sync() {
    // Umbrella invariant: after any deploy, the three sources of
    // truth (server state, local pin file, committed `als.toml.id`)
    // must agree on `(id, name)`. The check runs once after a fresh
    // deploy and again after a no-network skip, because that branch
    // takes a different code path (no `write_pin` call).
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let first = cli_for_home(&home_path)
        .arg(dir.path())
        .arg("--name")
        .arg("invariant-test")
        .assert()
        .success();
    let url = extract_url(&stdout_of(&first)).expect("URL");
    let id = site_id_from_url(&url).expect("id");
    assert_three_way_consistency(dir.path(), &home_path, &id, "invariant-test");

    // Second deploy, identical content → no-op skip path. The pin and
    // als.toml stay where they were; they must still match the
    // server's state.
    cli_for_home(&home_path).arg(dir.path()).assert().success();
    assert_three_way_consistency(dir.path(), &home_path, &id, "invariant-test");
}

#[test]
#[serial]
fn rename_via_deploy_keeps_id_consistent_across_all_sources() {
    // Companion to the test above: a rename-via-deploy
    // (`--name new` on top of a pinned site) leaves the id alone
    // everywhere but updates the name in all three sources.
    SERVER.reset();
    let dir = dir_with_one_file();
    let home = fresh_persistent_home();
    let home_path = home.path().to_path_buf();

    let first = cli_for_home(&home_path)
        .arg(dir.path())
        .arg("--name")
        .arg("v1")
        .assert()
        .success();
    let url = extract_url(&stdout_of(&first)).expect("URL");
    let id = site_id_from_url(&url).expect("id");
    assert_three_way_consistency(dir.path(), &home_path, &id, "v1");

    // Tip the digest so the second deploy actually hits the wire.
    fs::write(
        dir.path().join("index.html"),
        b"<!doctype html><title>v2</title>",
    )
    .expect("rewrite index");

    cli_for_home(&home_path)
        .arg(dir.path())
        .arg("--name")
        .arg("v2")
        .assert()
        .success();
    assert_three_way_consistency(dir.path(), &home_path, &id, "v2");
}

/// Assert the three sources of truth agree on `(id, name)`:
///
/// * `als.toml.id` in the source directory
/// * the local pin file under `~/.config/als/sites/`
/// * the server's `state.sites.get(id)` snapshot
fn assert_three_way_consistency(
    source_dir: &Path,
    home: &Path,
    expected_id: &str,
    expected_name: &str,
) {
    // 1. als.toml.id
    let toml_text =
        fs::read_to_string(source_dir.join("als.toml")).expect("als.toml must exist after deploy");
    assert!(
        toml_text.contains(&format!("id = \"{expected_id}\"")),
        "als.toml id mismatch (want {expected_id}); body:\n{toml_text}"
    );

    // 2. Local pin file — exactly one entry, mentioning the id and name.
    let pins = read_all_pins(home);
    assert!(
        pins.contains(expected_id) && pins.contains(expected_name),
        "pin store missing ({expected_id}, {expected_name}); got:\n{pins}"
    );

    // 3. Server state.
    let snap = fetch_state();
    let site = snap["sites"]
        .as_array()
        .and_then(|arr| arr.iter().find(|s| s["id"].as_str() == Some(expected_id)))
        .unwrap_or_else(|| panic!("server has no site with id {expected_id}: {snap}"));
    assert_eq!(
        site["projectName"].as_str(),
        Some(expected_name),
        "server name mismatch (want {expected_name}); site: {site}"
    );
}

/// Fetch the mock server's full state snapshot. Shared by the
/// three-way consistency checks and the cross-machine rm test.
fn fetch_state() -> Value {
    let url = SERVER.url.join("/__test/state").expect("join state");
    let body: Value = reqwest::blocking::Client::new()
        .get(url)
        .send()
        .expect("GET /__test/state")
        .json()
        .expect("parse state json");
    body.get("data").cloned().unwrap_or(body)
}

fn extract_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| {
            line.split_whitespace()
                .find(|tok| tok.starts_with("https://"))
        })
        .map(str::to_owned)
}

fn site_id_from_url(url: &str) -> Option<String> {
    // https://k7x2qm4j6p.a.ls → k7x2qm4j6p
    let rest = url.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    let id = host.split('.').next()?;
    Some(id.to_owned())
}
