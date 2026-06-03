//! Content fingerprint for `als <path>` no-op skip.
//!
//! [`digest`] returns a stable SHA-256 hex string over the input path's
//! *source content* (NOT the eventually-packed zip, which is full of
//! non-deterministic metadata like timestamps). The caller compares it
//! against the value recorded in `~/.config/als/sites/<name>.toml`; a hit
//! means the upload would push byte-identical input, so the deploy can
//! be short-circuited without running the pipeline.
//!
//! Algorithm
//!
//! * Single file → SHA-256 of the file's bytes.
//! * Directory → walk via [`crate::walk::walk`] under the standard
//!   ignore matcher (default ignores + `als.toml.exclude`), build a sorted
//!   list of `(relative_path, sha256(content))` pairs, then SHA-256 the
//!   serialised form `b"<path>\0<hex-hash>\n"` per entry. Sorting +
//!   per-entry SHA gives a deterministic fingerprint that's independent
//!   of walk order and resistant to two-file swap attacks.

use std::fs;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::ignore::build_matcher;
use als_core::Error;

const READ_BUF_BYTES: usize = 64 * 1024;

/// Compute a content fingerprint for `path`. See module docs for the
/// algorithm.
pub fn digest(path: &Path) -> Result<String, Error> {
    let meta = fs::metadata(path)
        .map_err(|e| Error::Other(format!("digest '{}': {e}", path.display())))?;
    if meta.is_file() {
        return digest_file(path);
    }
    if meta.is_dir() {
        return digest_directory(path);
    }
    Err(Error::Other(format!(
        "digest: '{}' is neither a regular file nor a directory",
        path.display()
    )))
}

fn digest_file(path: &Path) -> Result<String, Error> {
    let mut hasher = Sha256::new();
    let mut file = fs::File::open(path)
        .map_err(|e| Error::Other(format!("open '{}': {e}", path.display())))?;
    let mut buf = vec![0u8; READ_BUF_BYTES];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| Error::Other(format!("read '{}': {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn digest_directory(root: &Path) -> Result<String, Error> {
    let matcher = build_matcher(root)?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for raw in crate::walk::walk(root, &matcher) {
        let raw = raw?;
        let hash = digest_file(&raw.abs_path)?;
        entries.push((raw.rel_path.to_string_lossy().into_owned(), hash));
    }

    // `als.toml` is excluded from the bundle by `DEFAULT_IGNORES`, but
    // it changes what the bundle looks like (project `name`, viewer
    // `title`, `default_file`, `exclude`). Fold its bytes into the
    // fingerprint under a reserved key so any edit forces a re-deploy.
    let als_toml = root.join(crate::config::ALS_TOML);
    if als_toml.is_file() {
        entries.push((
            format!("__{}", crate::config::ALS_TOML),
            digest_file(&als_toml)?,
        ));
    }

    entries.sort();

    let mut top = Sha256::new();
    for (path, file_hash) in &entries {
        top.update(path.as_bytes());
        top.update(b"\0");
        top.update(file_hash.as_bytes());
        top.update(b"\n");
    }
    Ok(hex::encode(top.finalize()))
}

/// Minimal hex encoder. Internal so we keep the dep surface lean.
mod hex {
    pub(super) fn encode(bytes: impl AsRef<[u8]>) -> String {
        const ALPHABET: &[u8; 16] = b"0123456789abcdef";
        let bytes = bytes.as_ref();
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(ALPHABET[(b >> 4) as usize] as char);
            out.push(ALPHABET[(b & 0x0f) as usize] as char);
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn digest_of_single_file_is_stable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"hello world\n").unwrap();
        let a = digest(&path).unwrap();
        let b = digest(&path).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "sha256 hex = 64 chars");
    }

    #[test]
    fn digest_changes_when_file_content_changes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"first").unwrap();
        let a = digest(&path).unwrap();
        fs::write(&path, b"second").unwrap();
        let b = digest(&path).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn digest_of_directory_is_stable_and_path_aware() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"alpha").unwrap();
        fs::write(dir.path().join("b.txt"), b"beta").unwrap();
        let h1 = digest(dir.path()).unwrap();
        let h2 = digest(dir.path()).unwrap();
        assert_eq!(h1, h2);

        // Swapping the two files' contents (keeping the same byte set)
        // must produce a different hash — the per-entry path is part of
        // the fingerprint.
        fs::write(dir.path().join("a.txt"), b"beta").unwrap();
        fs::write(dir.path().join("b.txt"), b"alpha").unwrap();
        let h3 = digest(dir.path()).unwrap();
        assert_ne!(h1, h3);
    }

    #[test]
    fn digest_of_directory_drops_default_ignored_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("real.txt"), b"keep").unwrap();
        let baseline = digest(dir.path()).unwrap();

        // `.env` is in the default ignore set — adding it must not
        // change the digest.
        fs::write(dir.path().join(".env"), b"SECRET=1").unwrap();
        let after = digest(dir.path()).unwrap();
        assert_eq!(baseline, after);
    }

    #[test]
    fn digest_changes_when_als_toml_changes() {
        // als.toml is filtered from the bundle by the default ignore
        // set, but its bytes are folded into the fingerprint so any
        // edit (name / title / default_file / exclude) re-triggers
        // a deploy.
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), b"fn main() {}\n").unwrap();
        fs::write(dir.path().join("als.toml"), br#"name = "before""#).unwrap();
        let a = digest(dir.path()).unwrap();
        fs::write(dir.path().join("als.toml"), br#"name = "after""#).unwrap();
        let b = digest(dir.path()).unwrap();
        assert_ne!(a, b, "als.toml edit must move the fingerprint");
    }

    #[test]
    fn digest_of_directory_honours_als_toml_exclude() {
        // Both dirs carry an identical `als.toml` excluding `*.log` and
        // a `real.txt`; the "with noise" dir adds a `noise.log` the
        // exclude pattern drops. Digests must match — the matched file
        // is filtered. (als.toml itself is dropped by the default
        // ignore set, so it doesn't contribute to either digest.)
        let with_noise = tempdir().unwrap();
        fs::write(with_noise.path().join("real.txt"), b"keep").unwrap();
        fs::write(
            with_noise.path().join("als.toml"),
            br#"exclude = ["*.log"]"#,
        )
        .unwrap();
        fs::write(with_noise.path().join("noise.log"), b"junk").unwrap();
        let a = digest(with_noise.path()).unwrap();

        let clean = tempdir().unwrap();
        fs::write(clean.path().join("real.txt"), b"keep").unwrap();
        fs::write(clean.path().join("als.toml"), br#"exclude = ["*.log"]"#).unwrap();
        let b = digest(clean.path()).unwrap();

        assert_eq!(a, b, "noise.log should be filtered by als.toml exclude");
    }
}
