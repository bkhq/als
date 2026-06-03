//! Filtered directory walk that emits `Entry` records for non-ignored files.
//!
//! Uses `walkdir::WalkDir` with `follow_links(false)` so symlinks are never
//! traversed (no zip-bomb-by-link). Directory entries are dropped before the
//! iterator yields, and ignored directories are pruned via `filter_entry` so
//! their subtrees are never descended.

use std::path::{Component, Path, PathBuf};

use als_core::Error;
use ignore::gitignore::Gitignore;
use walkdir::WalkDir;

/// A single file selected for packing.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Absolute (or walk-root-relative) filesystem path used for reading.
    pub abs_path: PathBuf,
    /// Archive-relative path with `/` separators on every platform.
    pub rel_path: PathBuf,
    /// File size in bytes, used for the pre-zip archive size check.
    pub size: u64,
}

/// Walk `root` recursively and yield non-ignored files.
///
/// Errors surface as `Err` items in the iterator stream so the caller can
/// decide whether to short-circuit or accumulate.
pub fn walk<'a>(
    root: &'a Path,
    matcher: &'a Gitignore,
) -> impl Iterator<Item = Result<Entry, Error>> + 'a {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| {
            if entry.path() == root {
                return true;
            }
            let Ok(rel) = entry.path().strip_prefix(root) else {
                return false;
            };
            let is_dir = entry.file_type().is_dir();
            !matcher.matched(rel, is_dir).is_ignore()
        })
        .filter_map(move |res| match res {
            Ok(entry) => {
                if !entry.file_type().is_file() {
                    return None;
                }
                let rel = match entry.path().strip_prefix(root) {
                    Ok(p) => p,
                    Err(e) => {
                        return Some(Err(Error::Other(format!("strip_prefix: {e}"))));
                    }
                };
                let size = match entry.metadata() {
                    Ok(meta) => meta.len(),
                    Err(e) => {
                        return Some(Err(Error::Other(format!("metadata: {e}"))));
                    }
                };
                Some(Ok(Entry {
                    abs_path: entry.path().to_path_buf(),
                    rel_path: rel_with_forward_slashes(rel),
                    size,
                }))
            }
            Err(e) => {
                let msg = e.to_string();
                match e.into_io_error() {
                    Some(io) => Some(Err(Error::Io(io))),
                    None => Some(Err(Error::Other(format!("walk: {msg}")))),
                }
            }
        })
}

fn rel_with_forward_slashes(rel: &Path) -> PathBuf {
    let joined = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    PathBuf::from(joined)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::ignore::build_matcher;

    fn collect_rel(root: &Path) -> BTreeSet<String> {
        let matcher = build_matcher(root).unwrap();
        walk(root, &matcher)
            .map(|r| r.unwrap().rel_path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn yields_only_files() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.txt"), b"a").unwrap();
        fs::write(dir.path().join("sub/b.txt"), b"b").unwrap();

        let entries = collect_rel(dir.path());
        assert!(entries.contains("a.txt"));
        assert!(entries.contains("sub/b.txt"));
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn default_ignores_prune_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(dir.path().join(".git/HEAD"), b"ref: x").unwrap();
        fs::write(dir.path().join("node_modules/pkg/index.js"), b"//").unwrap();
        fs::write(dir.path().join("kept.txt"), b"kept").unwrap();
        fs::write(dir.path().join(".env"), b"SECRET=1").unwrap();
        fs::write(dir.path().join("server.pem"), b"---").unwrap();

        let entries = collect_rel(dir.path());
        assert_eq!(entries, BTreeSet::from(["kept.txt".to_owned()]));
    }

    #[test]
    fn nested_node_modules_pruned() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("frontend/node_modules/lodash")).unwrap();
        fs::create_dir_all(dir.path().join("apps/web/node_modules")).unwrap();
        fs::write(
            dir.path().join("frontend/node_modules/lodash/index.js"),
            b"// dep",
        )
        .unwrap();
        fs::write(dir.path().join("apps/web/node_modules/.keep"), b"").unwrap();
        fs::write(dir.path().join("frontend/index.html"), b"<html>").unwrap();
        fs::write(dir.path().join("apps/web/main.ts"), b"export {}").unwrap();

        let entries = collect_rel(dir.path());
        assert!(entries.contains("frontend/index.html"));
        assert!(entries.contains("apps/web/main.ts"));
        assert!(
            !entries.iter().any(|p| p.contains("node_modules")),
            "node_modules at any depth must be pruned, got: {entries:?}"
        );
    }

    #[test]
    fn als_toml_exclude_layered() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("als.toml"), br#"exclude = ["*.log"]"#).unwrap();
        fs::write(dir.path().join("keep.txt"), b"keep").unwrap();
        fs::write(dir.path().join("drop.log"), b"drop").unwrap();

        let entries = collect_rel(dir.path());
        assert!(entries.contains("keep.txt"));
        assert!(!entries.contains("drop.log"));
        // als.toml itself is also excluded by the default ignore set.
        assert!(!entries.contains("als.toml"));
    }
}
