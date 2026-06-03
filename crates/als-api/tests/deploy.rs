//! Integration test for `POST /api/deploy` against the Bun mock server.
//!
//! Drives the typed `deploy()` function end-to-end through `Client` and
//! asserts on the decoded `DeployResponse`. The `archive` payload is a
//! minimal byte blob — the mock does not validate zip structure.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::duration_suboptimal_units)]

mod common;

use std::path::PathBuf;
use std::time::Duration;

use als_api::{Client, DeployRequest, deploy};
use common::SERVER;
use secrecy::SecretString;
use serial_test::serial;
use tokio::runtime::Builder;

const DEFAULT_TOKEN: &str = "tk_test0001abcdef";

fn client_for_server(token: &str) -> Client {
    let cfg = als_core::Config {
        api: SERVER.url.clone(),
        token: SecretString::from(token.to_owned()),
        active_profile: None,
        source_path: PathBuf::from("/tmp/test.toml"),
    };
    Client::new(&cfg).expect("build Client")
}

fn run<F: std::future::Future>(fut: F) -> F::Output {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime")
        .block_on(fut)
}

#[test]
#[serial]
fn deploy_round_trip_returns_site_descriptor() {
    SERVER.reset();
    let client = client_for_server(DEFAULT_TOKEN);

    let archive: Vec<u8> = b"PK\x03\x04not-really-a-zip".to_vec();
    let request = DeployRequest {
        site_id: None,
        project_name: Some("hello-world".into()),
        expires: Some(als_core::Expires::Duration(Duration::from_secs(24 * 3_600))),
        auth_password: None,
    };

    let response =
        run(deploy(&client, archive, "site.zip".into(), request)).expect("deploy round-trip");

    assert!(!response.id.is_empty(), "id should be assigned");
    assert_eq!(response.project_name, "hello-world");
    assert!(!response.auth_required);
    assert!(
        response.url.starts_with("https://"),
        "url = {}",
        response.url
    );
    assert!(
        response.expires_at.is_some(),
        "expiresAt should be set when --expires given"
    );
}

#[test]
#[serial]
fn deploy_with_auth_password_flips_auth_required() {
    SERVER.reset();
    let client = client_for_server(DEFAULT_TOKEN);

    let request = DeployRequest {
        site_id: None,
        project_name: Some("secret-site".into()),
        expires: None,
        auth_password: Some("s3cret-xyz".into()),
    };
    let resp = run(deploy(
        &client,
        b"PK\x03\x04".to_vec(),
        "x.zip".into(),
        request,
    ))
    .expect("deploy");
    assert!(resp.auth_required);
}
