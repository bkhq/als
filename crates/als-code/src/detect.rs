//! Code-mode routing decision.
//!
//! [`classify`] returns `Some(CodeInput)` only when the input is
//! pure code — never for anything the existing pipeline can already
//! handle (HTML, markdown, mdbook, zip). The binary's `resolve_kind`
//! consults this *after* `als_md::classify` has already returned
//! `None`, so the only inputs reaching us are:
//!
//! - A single file (not `.md`, not `.html`, not `.zip`).
//! - A directory with no `book.toml`, no `*.md`, and no `index.html`
//!   at any depth.
//!
//! We additionally require that the directory contain at least one
//! file whose extension is a recognised code language. Pure asset
//! dumps (e.g. a folder of PNG files) fall back to the regular `site` path.

use std::path::Path;

use als_core::Error;

use crate::lang::is_recognized_code;
use crate::walker;

/// The classified input. Both variants borrow the original path.
#[derive(Debug, Clone, Copy)]
pub enum CodeInput<'a> {
    SingleFile(&'a Path),
    Directory(&'a Path),
}

impl CodeInput<'_> {
    pub fn path(&self) -> &Path {
        match self {
            CodeInput::SingleFile(p) | CodeInput::Directory(p) => p,
        }
    }
}

/// Wrap a path as a [`CodeInput`] without inspecting its shape.
///
/// Used by `--kind code` overrides: the user has already told us they
/// want the code-viewer rendering, so we skip the shape detection
/// (no `.html` / `.md` checks) and only enforce that the path itself
/// exists and is a regular file or directory.
pub fn force_input(path: &Path) -> Result<CodeInput<'_>, Error> {
    let meta = std::fs::metadata(path)
        .map_err(|e| Error::Other(format!("path '{}' not accessible: {e}", path.display())))?;
    if meta.is_dir() {
        return Ok(CodeInput::Directory(path));
    }
    if meta.is_file() {
        return Ok(CodeInput::SingleFile(path));
    }
    Err(Error::Other(format!(
        "code_unsupported_input: '{}' is neither a file nor a directory",
        path.display()
    )))
}

/// Decide whether `path` should be packaged as a code bundle.
///
/// Returns `Ok(None)` for paths that should fall through to the
/// existing als-pack pipeline (regular sites).
pub fn classify(path: &Path) -> Result<Option<CodeInput<'_>>, Error> {
    let meta = std::fs::metadata(path)
        .map_err(|e| Error::Other(format!("path '{}' not accessible: {e}", path.display())))?;

    if meta.is_file() {
        if is_recognized_code(path) {
            return Ok(Some(CodeInput::SingleFile(path)));
        }
        return Ok(None);
    }

    if !meta.is_dir() {
        return Ok(None);
    }

    if directory_is_code_shaped(path)? {
        Ok(Some(CodeInput::Directory(path)))
    } else {
        Ok(None)
    }
}

/// Decide whether `root` looks like a "directory of code" — i.e. has
/// no HTML / Markdown / mdbook hints anywhere under it, and at least
/// one file whose extension is recognised as code.
///
/// The walk uses the same default-ignore / `als.toml` matcher as
/// `als_pack`, so `node_modules` / `.git` etc. never disqualify
/// otherwise-legit code directories.
fn directory_is_code_shaped(root: &Path) -> Result<bool, Error> {
    if root.join("book.toml").is_file() {
        return Ok(false);
    }
    if root.join("index.html").is_file() {
        return Ok(false);
    }

    let matcher = als_pack::ignore::build_matcher(root)?;
    let mut saw_code = false;
    for entry in als_pack::walk::walk(root, &matcher) {
        let entry = entry?;
        let rel = &entry.rel_path;
        if let Some(ext) = rel
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
        {
            match ext.as_str() {
                "html" | "htm" | "md" | "markdown" => return Ok(false),
                _ => {}
            }
        }
        if rel.file_name().and_then(|s| s.to_str()) == Some("book.toml") {
            return Ok(false);
        }
        if !saw_code && is_recognized_code(rel) {
            saw_code = true;
        }
    }
    Ok(saw_code)
}

/// Tighten the per-input limits before bundling. Called from
/// `bundle::bundle` so the binary surfaces a clear error before
/// attempting any filesystem work.
pub(crate) fn validate_limits(entries: &[walker::Entry]) -> Result<(), Error> {
    if entries.is_empty() {
        return Err(Error::Other(
            "code_no_files: no code files after ignore rules".into(),
        ));
    }
    if entries.len() > crate::MAX_FILES {
        return Err(Error::Other(format!(
            "code_too_many_files: {} files exceed limit {}",
            entries.len(),
            crate::MAX_FILES
        )));
    }
    let mut total: u64 = 0;
    for e in entries {
        if e.size > crate::MAX_FILE_BYTES {
            return Err(Error::Other(format!(
                "code_file_too_large: {} is {} bytes > {} byte limit",
                e.rel_path.display(),
                e.size,
                crate::MAX_FILE_BYTES
            )));
        }
        total = total.saturating_add(e.size);
    }
    if total > crate::MAX_TOTAL_BYTES {
        return Err(Error::Other(format!(
            "code_total_too_large: {total} bytes > {} byte limit",
            crate::MAX_TOTAL_BYTES
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn single_code_file_classifies() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("main.rs");
        fs::write(&file, b"fn main() {}\n").unwrap();
        let out = classify(&file).unwrap();
        assert!(matches!(out, Some(CodeInput::SingleFile(_))));
    }

    #[test]
    fn single_unknown_file_returns_none() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("photo.png");
        fs::write(&file, b"\x89PNG").unwrap();
        assert!(classify(&file).unwrap().is_none());
    }

    #[test]
    fn directory_of_code_classifies() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), b"[package]\n").unwrap();
        fs::write(dir.path().join("src/main.rs"), b"fn main() {}\n").unwrap();
        let out = classify(dir.path()).unwrap();
        assert!(matches!(out, Some(CodeInput::Directory(_))));
    }

    #[test]
    fn directory_with_index_html_returns_none() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();
        fs::write(dir.path().join("main.js"), b"// js\n").unwrap();
        assert!(classify(dir.path()).unwrap().is_none());
    }

    #[test]
    fn directory_with_nested_html_returns_none() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("public")).unwrap();
        fs::write(dir.path().join("public/about.html"), b"<html></html>").unwrap();
        fs::write(dir.path().join("main.js"), b"// js\n").unwrap();
        assert!(classify(dir.path()).unwrap().is_none());
    }

    #[test]
    fn directory_with_md_returns_none() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("README.md"), b"# hi\n").unwrap();
        fs::write(dir.path().join("main.rs"), b"fn main() {}\n").unwrap();
        assert!(classify(dir.path()).unwrap().is_none());
    }

    #[test]
    fn directory_with_book_toml_returns_none() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("book.toml"), b"[book]\ntitle = \"x\"\n").unwrap();
        fs::write(dir.path().join("main.rs"), b"fn main() {}\n").unwrap();
        assert!(classify(dir.path()).unwrap().is_none());
    }

    #[test]
    fn directory_of_pure_assets_returns_none() {
        // No recognised code extension anywhere — fall back to plain site.
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("photo.png"), b"\x89PNG").unwrap();
        fs::write(dir.path().join("video.mp4"), b"fake").unwrap();
        assert!(classify(dir.path()).unwrap().is_none());
    }

    #[test]
    fn ignored_subtrees_do_not_disqualify_code_dir() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(
            dir.path().join("node_modules/pkg/index.html"),
            b"<html></html>",
        )
        .unwrap();
        fs::write(dir.path().join("main.ts"), b"export {};\n").unwrap();
        let out = classify(dir.path()).unwrap();
        assert!(
            matches!(out, Some(CodeInput::Directory(_))),
            "node_modules HTML must not disqualify a code directory",
        );
    }

    #[test]
    fn missing_path_errors() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("ghost");
        assert!(classify(&missing).is_err());
    }
}
