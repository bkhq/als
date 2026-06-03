//! Request-URL -> on-disk path resolution.
//!
//! Centralises the rules so the request handler stays a thin shell:
//!
//! 1. Drop query string and fragment.
//! 2. Percent-decode the path.
//! 3. Strip leading `/`.
//! 4. Refuse paths containing `..` segments before resolution (the
//!    `canonicalize` check below is the real safety net, but rejecting
//!    here means we don't even touch the filesystem on obvious abuse).
//! 5. Join under `root`, canonicalize, and reject if the canonical
//!    result does not start with `root.canonicalize()` (zip-slip
//!    equivalent for HTTP).
//! 6. If the result is a directory, look for `index.html` inside.

use std::path::{Component, Path, PathBuf};

pub(crate) fn resolve(root: &Path, url: &str) -> Option<PathBuf> {
    let url_path = url.split(['?', '#']).next().unwrap_or("");
    let decoded = percent_decode(url_path);
    let trimmed = decoded.trim_start_matches('/');

    let mut accum = PathBuf::new();
    for segment in Path::new(trimmed).components() {
        match segment {
            Component::Normal(s) => accum.push(s),
            Component::CurDir => {}
            // `..`, prefixes, root anchors are all rejected up front.
            _ => return None,
        }
    }

    let candidate = root.join(&accum);
    let canonical_candidate = candidate.canonicalize().ok()?;
    let canonical_root = root.canonicalize().ok()?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return None;
    }

    if canonical_candidate.is_dir() {
        let index = canonical_candidate.join("index.html");
        return index.is_file().then_some(index);
    }
    canonical_candidate.is_file().then_some(canonical_candidate)
}

/// Percent-decode `s` into a `String`. Invalid percent sequences
/// (`%` not followed by two hex digits) are passed through verbatim
/// to mirror how browsers handle malformed URLs.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Lossy is fine: bytes that don't form valid UTF-8 become U+FFFD
    // and the canonicalize step will fail to find a matching file
    // (which 404s cleanly).
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.html"), b"<html>root</html>").unwrap();
        fs::create_dir(dir.path().join("assets")).unwrap();
        fs::write(dir.path().join("assets/logo.svg"), b"<svg/>").unwrap();
        fs::create_dir(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("docs/index.html"), b"<html>docs</html>").unwrap();
        // file in a subdir with a space + percent in name
        fs::write(dir.path().join("assets/a b.png"), b"\x89PNG").unwrap();
        dir
    }

    #[test]
    fn slash_resolves_to_root_index() {
        let dir = fixture();
        let resolved = resolve(dir.path(), "/").unwrap();
        assert_eq!(
            resolved,
            dir.path().canonicalize().unwrap().join("index.html")
        );
    }

    #[test]
    fn subdir_resolves_to_index_in_subdir() {
        let dir = fixture();
        let resolved = resolve(dir.path(), "/docs/").unwrap();
        assert_eq!(
            resolved,
            dir.path().canonicalize().unwrap().join("docs/index.html")
        );
    }

    #[test]
    fn explicit_asset_path_resolves() {
        let dir = fixture();
        let resolved = resolve(dir.path(), "/assets/logo.svg").unwrap();
        assert!(resolved.ends_with("assets/logo.svg"));
    }

    #[test]
    fn percent_decoded_path_resolves() {
        let dir = fixture();
        // `%20` -> space
        let resolved = resolve(dir.path(), "/assets/a%20b.png").unwrap();
        assert!(resolved.ends_with("assets/a b.png"));
    }

    #[test]
    fn query_string_is_ignored() {
        let dir = fixture();
        let resolved = resolve(dir.path(), "/assets/logo.svg?v=42").unwrap();
        assert!(resolved.ends_with("assets/logo.svg"));
    }

    #[test]
    fn parent_traversal_is_rejected_pre_resolution() {
        let dir = fixture();
        assert!(resolve(dir.path(), "/../etc/passwd").is_none());
        assert!(resolve(dir.path(), "/docs/../../etc/passwd").is_none());
    }

    #[test]
    fn missing_file_returns_none() {
        let dir = fixture();
        assert!(resolve(dir.path(), "/no-such-file.html").is_none());
    }

    #[test]
    fn directory_without_index_returns_none() {
        let dir = fixture();
        fs::create_dir(dir.path().join("empty")).unwrap();
        assert!(resolve(dir.path(), "/empty/").is_none());
    }
}
