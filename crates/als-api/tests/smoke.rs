//! Smoke test: confirm the harness spawns the Bun mock server and that
//! `SERVER.reset()` completes without panicking. Skipped automatically by
//! the panic if `bun` is not on PATH; running it is gated by Bun availability.

mod common;

use common::SERVER;
use serial_test::serial;

#[test]
#[serial]
fn server_responds_to_reset() {
    SERVER.reset();
}
