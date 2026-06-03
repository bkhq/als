//! Markdown + mdbook pre-render layer for `als <path>`.
//!
//! Two-stage flow:
//!
//! 1. `classify(path)` inspects the input. It returns `Some` for:
//!    - a directory containing `book.toml` ([`MdInput::MdBook`] — author
//!      manages SUMMARY.md / book.toml themselves);
//!    - a directory containing at least one `*.md` after `als.toml`
//!      exclude filtering ([`MdInput::AutoMdBook`] — als auto-bootstraps
//!      `book.toml` + `SUMMARY.md` from the file tree);
//!    - a single `*.md` file (also [`MdInput::AutoMdBook`], wrapped
//!      around the lone file).
//!
//!    Anything else returns `None` and the binary falls through to
//!    `als-pack` for the existing folder / zip / single-HTML paths.
//!
//! 2. `render(input)` produces a [`Rendered`] handle: an owning temp
//!    directory containing plain HTML that the binary then hands to
//!    `als-pack`. Both `MdInput` variants ultimately drive
//!    `mdbook::MDBook::build` so output styling, navigation, and search
//!    are uniform regardless of whether the user authored `book.toml`
//!    themselves.

mod auto_book;

use std::path::{Path, PathBuf};

use als_core::Error;
use tempfile::TempDir;

/// A classified Markdown / mdbook input. Returned by [`classify`].
#[derive(Debug)]
pub enum MdInput<'a> {
    /// Directory that already contains `book.toml` at its root.
    MdBook(&'a Path),
    /// Markdown source that needs `book.toml` + `SUMMARY.md` generated
    /// before mdbook can consume it.
    AutoMdBook(AutoMdBookSource<'a>),
}

/// Shape of the auto-bootstrap input.
#[derive(Debug)]
pub enum AutoMdBookSource<'a> {
    /// Directory of `*.md` (and optional asset) files, no `book.toml`.
    Directory(&'a Path),
    /// Single `*.md` file.
    File(&'a Path),
}

/// Output of [`render`]: an owning temp directory of HTML, plus the
/// suggested archive stem the binary surfaces in the upload log.
pub struct Rendered {
    dir: TempDir,
    /// Suggested archive basename (without `.zip`).
    pub stem: String,
}

impl Rendered {
    /// Borrow the rendered tree root as a `Path`.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// Inspect `path` and return a non-`None` [`MdInput`] when the binary
/// should pre-render through this crate before invoking `als-pack`.
pub fn classify(path: &Path) -> Result<Option<MdInput<'_>>, Error> {
    let meta = std::fs::metadata(path)
        .map_err(|e| Error::Other(format!("path '{}' not accessible: {e}", path.display())))?;

    if meta.is_dir() {
        if path.join("book.toml").is_file() {
            return Ok(Some(MdInput::MdBook(path)));
        }
        if directory_contains_markdown(path)? {
            return Ok(Some(MdInput::AutoMdBook(AutoMdBookSource::Directory(path))));
        }
        return Ok(None);
    }
    if !meta.is_file() {
        return Ok(None);
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("md")) {
        return Ok(Some(MdInput::AutoMdBook(AutoMdBookSource::File(path))));
    }
    Ok(None)
}

/// Render `input` into a fresh temp directory and return the resulting
/// [`Rendered`] handle.
pub fn render(input: MdInput<'_>) -> Result<Rendered, Error> {
    let out = tempfile::tempdir().map_err(|e| Error::Other(format!("create out dir: {e}")))?;
    let stem = match input {
        MdInput::MdBook(root) => {
            run_mdbook(root, out.path())?;
            basename_or_default(root, "site")
        }
        MdInput::AutoMdBook(source) => {
            let scratch = tempfile::tempdir()
                .map_err(|e| Error::Other(format!("create scratch dir: {e}")))?;
            match source {
                AutoMdBookSource::Directory(root) => {
                    auto_book::bootstrap_from_dir(root, scratch.path())?;
                    run_mdbook(scratch.path(), out.path())?;
                    basename_or_default(root, "site")
                }
                AutoMdBookSource::File(file) => {
                    auto_book::bootstrap_from_file(file, scratch.path())?;
                    run_mdbook(scratch.path(), out.path())?;
                    file_stem_or_default(file)
                }
            }
            // `scratch` drops here; its contents are not needed once the
            // mdbook build wrote into `out`.
        }
    };
    Ok(Rendered { dir: out, stem })
}

fn run_mdbook(root: &Path, out: &Path) -> Result<(), Error> {
    let mut book =
        mdbook::MDBook::load(root).map_err(|e| Error::Other(format!("mdbook load: {e}")))?;
    book.config.build.build_dir = out.to_path_buf();
    book.build()
        .map_err(|e| Error::Other(format!("mdbook build: {e}")))?;
    Ok(())
}

fn directory_contains_markdown(root: &Path) -> Result<bool, Error> {
    let matcher = als_pack::ignore::build_matcher(root)?;
    for entry in als_pack::walk::walk(root, &matcher) {
        let entry = entry?;
        if let Some(ext) = entry.rel_path.extension().and_then(|s| s.to_str())
            && ext.eq_ignore_ascii_case("md")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn basename_or_default(root: &Path, fallback: &str) -> String {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root));
    canonical
        .file_name()
        .map_or_else(|| fallback.to_owned(), |n| n.to_string_lossy().into_owned())
}

fn file_stem_or_default(file: &Path) -> String {
    file.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("page")
        .to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn classify_returns_none_for_plain_html_dir() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();
        fs::write(dir.path().join("style.css"), b"body{}").unwrap();
        assert!(classify(dir.path()).unwrap().is_none());
    }

    #[test]
    fn classify_detects_mdbook() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("book.toml"), "[book]\ntitle = \"x\"\n").unwrap();
        match classify(dir.path()).unwrap() {
            Some(MdInput::MdBook(_)) => {}
            other => panic!("expected MdBook, got {other:?}"),
        }
    }

    #[test]
    fn classify_detects_markdown_dir_as_auto() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("notes.md"), b"# notes\n").unwrap();
        match classify(dir.path()).unwrap() {
            Some(MdInput::AutoMdBook(AutoMdBookSource::Directory(_))) => {}
            other => panic!("expected AutoMdBook::Directory, got {other:?}"),
        }
    }

    #[test]
    fn classify_detects_single_md_file_as_auto() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("readme.md");
        fs::write(&f, b"# r\n").unwrap();
        match classify(&f).unwrap() {
            Some(MdInput::AutoMdBook(AutoMdBookSource::File(_))) => {}
            other => panic!("expected AutoMdBook::File, got {other:?}"),
        }
    }

    #[test]
    fn classify_returns_none_for_zip_file() {
        let dir = tempdir().unwrap();
        let z = dir.path().join("a.zip");
        fs::write(&z, b"PK").unwrap();
        assert!(classify(&z).unwrap().is_none());
    }

    #[test]
    fn render_single_md_produces_book_with_index() {
        let src = tempdir().unwrap();
        let file = src.path().join("report.md");
        fs::write(&file, "# Q3 Report\n\nBody.\n").unwrap();

        let rendered = render(MdInput::AutoMdBook(AutoMdBookSource::File(&file))).unwrap();
        assert_eq!(rendered.stem, "report");
        // mdbook always emits `index.html` at the book root.
        let index = rendered.path().join("index.html");
        assert!(index.is_file(), "{index:?} should exist");
        let html = fs::read_to_string(&index).unwrap();
        // The page title from H1 surfaces somewhere in the rendered HTML.
        assert!(
            html.contains("Q3 Report"),
            "rendered html should contain the H1 title"
        );
    }

    #[test]
    fn render_markdown_dir_produces_book_with_pages() {
        let src = tempdir().unwrap();
        fs::write(src.path().join("index.md"), "# Home\n\nHi.\n").unwrap();
        fs::write(src.path().join("other.md"), "# Other\n\nMore.\n").unwrap();

        let rendered =
            render(MdInput::AutoMdBook(AutoMdBookSource::Directory(src.path()))).unwrap();
        assert!(rendered.path().join("index.html").is_file());
        assert!(rendered.path().join("other.html").is_file());
    }
}
