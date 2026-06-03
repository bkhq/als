//! Bounded directory walk for the code-mode bundle.
//!
//! Delegates the gitignore matcher to
//! [`als_pack::ignore::build_matcher`] (which already merges the
//! default ignore set with `als.toml.exclude` and drops `als.toml`
//! itself), then layers code-mode specific caps:
//!
//! - depth cut-off ([`crate::MAX_DEPTH`])
//! - binary file sniffer
//! - per-file size guard ([`crate::MAX_FILE_BYTES`])
//!
//! Total-count and total-byte caps are checked by
//! [`crate::detect::validate_limits`] after the walk completes, so
//! they apply to the actual included set rather than to whatever
//! `walkdir` happened to visit before erroring.

use std::path::{Path, PathBuf};

use als_core::Error;

/// One file scheduled for inclusion in the bundle.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    /// Absolute filesystem path used for reading.
    pub(crate) abs_path: PathBuf,
    /// Path relative to the walk root, with `/` separators.
    pub(crate) rel_path: PathBuf,
    /// Byte size as returned by metadata.
    pub(crate) size: u64,
}

/// Walk `root` and return every kept file. Binary files are skipped
/// silently; per-file size limits and depth limits surface as errors.
pub(crate) fn walk(root: &Path) -> Result<Vec<Entry>, Error> {
    let matcher = als_pack::ignore::build_matcher(root)?;
    let mut entries = Vec::new();

    for raw in als_pack::walk::walk(root, &matcher) {
        let raw = raw?;
        let depth = raw.rel_path.components().count();
        if depth > crate::MAX_DEPTH {
            return Err(Error::Other(format!(
                "code_depth_exceeded: '{}' is {} levels deep (limit {})",
                raw.rel_path.display(),
                depth,
                crate::MAX_DEPTH
            )));
        }
        if raw.size > crate::MAX_FILE_BYTES {
            return Err(Error::Other(format!(
                "code_file_too_large: '{}' is {} bytes > {} byte limit",
                raw.rel_path.display(),
                raw.size,
                crate::MAX_FILE_BYTES
            )));
        }
        if is_probably_binary(&raw.abs_path)? {
            continue;
        }
        entries.push(Entry {
            abs_path: raw.abs_path,
            rel_path: raw.rel_path,
            size: raw.size,
        });
    }

    entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(entries)
}

/// Wrap a single file path as a one-entry result. Mirrors the walk
/// invariants so single-file inputs reuse the same downstream code.
pub(crate) fn single(path: &Path) -> Result<Vec<Entry>, Error> {
    let meta = std::fs::metadata(path)?;
    let size = meta.len();
    if size > crate::MAX_FILE_BYTES {
        return Err(Error::Other(format!(
            "code_file_too_large: '{}' is {} bytes > {} byte limit",
            path.display(),
            size,
            crate::MAX_FILE_BYTES
        )));
    }
    let rel = PathBuf::from(
        path.file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Other("single-file path has no name".into()))?,
    );
    Ok(vec![Entry {
        abs_path: path.to_path_buf(),
        rel_path: rel,
        size,
    }])
}

/// Heuristic binary detector. Reads up to 1024 bytes and returns
/// `true` if a NUL byte is present or > 30% of bytes are outside the
/// printable / whitespace ASCII range.
///
/// Avoids `mime_guess` / `infer` to keep the dep graph pure-Rust and
/// minimal; the heuristic is fine for the small text inputs code-mode
/// targets.
fn is_probably_binary(path: &Path) -> Result<bool, Error> {
    use std::io::Read;

    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 1024];
    let n = f.read(&mut buf)?;
    if n == 0 {
        return Ok(false);
    }
    let slice = &buf[..n];
    if slice.contains(&0u8) {
        return Ok(true);
    }
    let weird = slice
        .iter()
        .filter(|&&b| b < 0x09 || (b > 0x0d && b < 0x20))
        .count();
    Ok(weird * 100 / n > 30)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn rel_set(entries: &[Entry]) -> BTreeSet<String> {
        entries
            .iter()
            .map(|e| e.rel_path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn walk_collects_text_files() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), b"[package]\n").unwrap();
        fs::write(dir.path().join("src/main.rs"), b"fn main() {}\n").unwrap();
        let entries = walk(dir.path()).unwrap();
        let names = rel_set(&entries);
        assert!(names.contains("Cargo.toml"));
        assert!(names.contains("src/main.rs"));
    }

    #[test]
    fn walk_skips_binaries() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("ok.rs"), b"fn main() {}\n").unwrap();
        // PNG-ish magic with NUL bytes — should be flagged binary.
        fs::write(
            dir.path().join("photo.png"),
            b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0d",
        )
        .unwrap();
        let entries = walk(dir.path()).unwrap();
        let names = rel_set(&entries);
        assert!(names.contains("ok.rs"));
        assert!(!names.contains("photo.png"));
    }

    #[test]
    fn walk_respects_default_ignores() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(dir.path().join("node_modules/pkg/index.js"), b"// dep").unwrap();
        fs::write(dir.path().join("main.rs"), b"fn main() {}\n").unwrap();
        let entries = walk(dir.path()).unwrap();
        let names = rel_set(&entries);
        assert!(names.contains("main.rs"));
        assert!(!names.iter().any(|n| n.contains("node_modules")));
    }

    #[test]
    fn walk_errors_on_oversize_file() {
        let dir = tempdir().unwrap();
        let big = dir.path().join("big.txt");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(crate::MAX_FILE_BYTES + 1).unwrap();
        let err = walk(dir.path()).unwrap_err();
        match err {
            Error::Other(msg) => {
                assert!(msg.starts_with("code_file_too_large"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn walk_errors_when_too_deep() {
        let dir = tempdir().unwrap();
        let mut cursor = dir.path().to_path_buf();
        for i in 0..=crate::MAX_DEPTH {
            cursor = cursor.join(format!("l{i}"));
        }
        fs::create_dir_all(&cursor).unwrap();
        fs::write(cursor.join("buried.rs"), b"fn main() {}\n").unwrap();
        let err = walk(dir.path()).unwrap_err();
        match err {
            Error::Other(msg) => assert!(msg.starts_with("code_depth_exceeded"), "got: {msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn single_wraps_one_file() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("a.ts");
        fs::write(&f, b"export {};").unwrap();
        let entries = single(&f).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].rel_path.to_string_lossy(), "a.ts");
    }

    #[test]
    fn single_rejects_oversize() {
        let dir = tempdir().unwrap();
        let big = dir.path().join("big.txt");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(crate::MAX_FILE_BYTES + 1).unwrap();
        assert!(single(&big).is_err());
    }

    #[test]
    fn walk_skips_als_toml() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("als.toml"), b"").unwrap();
        fs::write(dir.path().join("main.rs"), b"fn main() {}\n").unwrap();
        let entries = walk(dir.path()).unwrap();
        let names = rel_set(&entries);
        assert!(names.contains("main.rs"));
        assert!(
            !names.contains("als.toml"),
            "als.toml must not leak into the bundle",
        );
    }

    #[test]
    fn walk_als_toml_excludes_are_honored() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), b"fn main() {}\n").unwrap();
        fs::write(dir.path().join("scratch.rs"), b"// throwaway\n").unwrap();
        fs::create_dir_all(dir.path().join("notes")).unwrap();
        fs::write(dir.path().join("notes/todo.rs"), b"// later\n").unwrap();
        fs::write(
            dir.path().join("als.toml"),
            br#"exclude = ["scratch.rs", "notes/**"]"#,
        )
        .unwrap();

        let entries = walk(dir.path()).unwrap();
        let names = rel_set(&entries);
        assert!(names.contains("main.rs"));
        assert!(!names.contains("scratch.rs"));
        assert!(!names.iter().any(|n| n.starts_with("notes/")));
    }
}
