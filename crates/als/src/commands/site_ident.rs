//! Resolve a `<id|name>` argument to a canonical site id.
//!
//! 1. If `arg` matches `^[a-z2-7]{10}$` it is treated as a site id and
//!    probed via `GET sites/:id`. On 404 the resolver falls through to
//!    step 2.
//! 2. Otherwise a single `GET sites?limit=200` is issued and the
//!    response is scanned for `projectName == arg` (case-sensitive).
//!    Exactly one match → that id. Zero → `Error::Other("not found")`.
//!    Multiple → `Error::Other("ambiguous: matched ids …")`.
//!
//! The single-page lookup is intentional. Users with more than 200 sites
//! must pass the id directly; the error message says so.

use als_api::endpoints::sites;
use als_api::{ApiError, Client, ListSitesQuery};
use als_core::Error;

const ID_LIMIT: u32 = 200;
const ID_LEN: usize = 10;

fn looks_like_id(arg: &str) -> bool {
    if arg.len() != ID_LEN {
        return false;
    }
    arg.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'))
}

pub(crate) async fn resolve(client: &Client, arg: &str) -> Result<String, Error> {
    if looks_like_id(arg) {
        match sites::get(client, arg).await {
            Ok(_) => return Ok(arg.to_owned()),
            Err(ApiError::NotFound) => {
                // Fall through to name lookup.
            }
            Err(other) => return Err(other.into()),
        }
    }
    resolve_by_name(client, arg).await
}

async fn resolve_by_name(client: &Client, arg: &str) -> Result<String, Error> {
    let query = ListSitesQuery {
        limit: Some(ID_LIMIT),
        page: Some(1),
        all: false,
    };
    let resp = sites::list(client, &query).await?;
    let matches: Vec<&str> = resp
        .data
        .iter()
        .filter(|s| s.project_name == arg)
        .map(|s| s.id.as_str())
        .collect();
    match matches.len() {
        0 => Err(Error::Other(format!(
            "no site with id or name \"{arg}\" (first {ID_LIMIT} sites scanned; pass the id directly if you have more)"
        ))),
        1 => Ok(matches[0].to_owned()),
        _ => Err(Error::Other(format!(
            "ambiguous: name \"{arg}\" matches {} sites (ids: {}). Pass an id directly.",
            matches.len(),
            matches.join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_id_accepts_base32_lower() {
        assert!(looks_like_id("k7x2qm4j6p"));
        assert!(looks_like_id("abcdefghij"));
        assert!(looks_like_id("23456abcde"));
    }

    #[test]
    fn looks_like_id_rejects_wrong_length() {
        assert!(!looks_like_id("abcde")); // 5 chars — old id length
        assert!(!looks_like_id("abcdefghi")); // 9 chars
        assert!(!looks_like_id("abcdefghijk")); // 11 chars
        assert!(!looks_like_id(""));
    }

    #[test]
    fn looks_like_id_rejects_outside_alphabet() {
        // 0/1/8/9 are outside the base32 lower [a-z2-7] alphabet.
        assert!(!looks_like_id("0bcdefghij"));
        assert!(!looks_like_id("abc1efghij"));
        assert!(!looks_like_id("abc-efghij"));
        assert!(!looks_like_id("ABCDEFGHIJ"));
    }
}
