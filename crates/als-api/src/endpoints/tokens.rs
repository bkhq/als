//! `DELETE tokens/:prefix` — revoke an API token by its prefix.
//!
//! The CLI never lists or creates tokens through this typed client;
//! those flows happen via the console UI or the pairing handshake.
//! `delete` is used by `als auth revoke` to revoke the token saved
//! in `config.toml`.

use crate::Client;
use crate::error::ApiError;

/// Revoke the token identified by `prefix`. Returns `Ok(())` on a
/// 200/204 success; `ApiError::NotFound` if the prefix is unknown to
/// the authenticated user.
pub async fn delete(client: &Client, prefix: &str) -> Result<(), ApiError> {
    let path = format!("tokens/{prefix}");
    client.delete(&path).await
}
