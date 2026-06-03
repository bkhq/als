//! als CLI packaging: ignore filtering, directory walk, zip writer.
//!
//! Three input shapes are accepted by [`classify`] per `docs/cli-spec.md`:
//! a directory (recursive walk + zip), a `.zip` file (pass-through), or a
//! single `.html` file (wrapped as `<stem>/index.html` inside a new zip).
//! Anything else is rejected.

pub mod config;
pub mod digest;
pub mod ignore;
pub mod walk;
pub mod zip;

pub use config::{ALS_TOML, AlsConfig};
pub use digest::digest;

use std::io::Cursor;
use std::path::{Path, PathBuf};

use als_core::Error;

pub use crate::walk::Entry;

/// Maximum archive size accepted by the server. Mirrors the v1 limit
/// documented in `docs/cli-spec.md`.
pub const MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024;

/// Classified input ready to feed [`pack`].
#[derive(Debug, Clone, Copy)]
pub enum PackInput<'a> {
    Directory(&'a Path),
    Zip(&'a Path),
    SingleHtml(&'a Path),
}

/// Result of [`pack`]: in-memory archive plus metadata for progress display.
#[derive(Debug, Clone)]
pub struct Packed {
    pub bytes: Vec<u8>,
    pub file_count: u32,
    pub uncompressed_size: u64,
    pub display_filename: String,
}

/// Classify a user-provided path into a supported [`PackInput`].
pub fn classify(path: &Path) -> Result<PackInput<'_>, Error> {
    let meta = std::fs::metadata(path)
        .map_err(|e| Error::Other(format!("path '{}' not accessible: {e}", path.display())))?;
    if meta.is_dir() {
        return Ok(PackInput::Directory(path));
    }
    if !meta.is_file() {
        return Err(Error::Other(format!(
            "path '{}' is not a regular file or directory",
            path.display()
        )));
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("zip") => Ok(PackInput::Zip(path)),
        Some("html") => Ok(PackInput::SingleHtml(path)),
        _ => Err(Error::Other(
            "only directories, .zip, or .html accepted".into(),
        )),
    }
}

/// Produce a [`Packed`] archive for the given input.
pub fn pack(input: PackInput<'_>) -> Result<Packed, Error> {
    match input {
        PackInput::Directory(root) => pack_directory(root),
        PackInput::Zip(path) => pack_zip_passthrough(path),
        PackInput::SingleHtml(path) => pack_single_html(path),
    }
}

fn pack_directory(root: &Path) -> Result<Packed, Error> {
    let matcher = ignore::build_matcher(root)?;
    let entries: Vec<Entry> = walk::walk(root, &matcher).collect::<Result<_, _>>()?;
    let uncompressed_size: u64 = entries.iter().map(|e| e.size).sum();
    let file_count =
        u32::try_from(entries.len()).map_err(|_| Error::Other("file count exceeds u32".into()))?;
    let bytes = zip::write_zip(entries)?;
    // Size cap is on the *compressed* archive — same surface the
    // server validates and the same number that determines upload
    // bandwidth. Pre-zip uncompressed sums are only kept for the
    // progress display.
    check_archive_size(bytes.len() as u64)?;
    Ok(Packed {
        bytes,
        file_count,
        uncompressed_size,
        display_filename: format!("{}.zip", basename_or_default(root)),
    })
}

fn pack_zip_passthrough(path: &Path) -> Result<Packed, Error> {
    let meta = std::fs::metadata(path)?;
    let size = meta.len();
    check_archive_size(size)?;
    let bytes = std::fs::read(path)?;
    let mut archive = ::zip::ZipArchive::new(Cursor::new(&bytes[..]))
        .map_err(|e| Error::Other(format!("invalid zip '{}': {e}", path.display())))?;
    let file_count =
        u32::try_from(archive.len()).map_err(|_| Error::Other("file count exceeds u32".into()))?;
    let mut uncompressed_size: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| Error::Other(format!("read zip entry {i}: {e}")))?;
        // Defensive: reject zip-slip / absolute paths client-side so a
        // hand-built upstream zip can never reach the deploy endpoint.
        // The server enforces the same invariant, but failing locally
        // surfaces a clearer error and skips the upload cost.
        if entry.enclosed_name().is_none() {
            return Err(Error::Other(format!(
                "unsafe path in zip entry {i}: {:?}",
                entry.name()
            )));
        }
        uncompressed_size = uncompressed_size.saturating_add(entry.size());
    }
    let display_filename = path.file_name().map_or_else(
        || "archive.zip".into(),
        |n| n.to_string_lossy().into_owned(),
    );
    Ok(Packed {
        bytes,
        file_count,
        uncompressed_size,
        display_filename,
    })
}

/// Reject any archive whose final byte size exceeds [`MAX_ARCHIVE_BYTES`].
/// Pulled out so the per-mode size policies stay identical and so the
/// unit tests can exercise the bound without writing tens of megabytes
/// of incompressible bytes to a temp directory.
fn check_archive_size(size: u64) -> Result<(), Error> {
    if size > MAX_ARCHIVE_BYTES {
        return Err(too_large(size));
    }
    Ok(())
}

fn pack_single_html(path: &Path) -> Result<Packed, Error> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::Other("html filename has no usable stem".into()))?;
    if stem.is_empty() {
        return Err(Error::Other("html filename stem is empty".into()));
    }
    let size = std::fs::metadata(path)?.len();
    if size > MAX_ARCHIVE_BYTES {
        return Err(too_large(size));
    }
    let entries = vec![Entry {
        abs_path: path.to_path_buf(),
        rel_path: PathBuf::from(format!("{stem}/index.html")),
        size,
    }];
    let bytes = zip::write_zip(entries)?;
    Ok(Packed {
        bytes,
        file_count: 1,
        uncompressed_size: size,
        display_filename: format!("{stem}.zip"),
    })
}

fn basename_or_default(root: &Path) -> String {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    canonical
        .file_name()
        .map_or_else(|| "archive".into(), |n| n.to_string_lossy().into_owned())
}

fn too_large(size: u64) -> Error {
    Error::Other(format!(
        "archive too large: {size} bytes exceeds {MAX_ARCHIVE_BYTES} byte limit"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::fs;
    use std::io::Read;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn classify_directory() {
        let dir = tempdir().unwrap();
        let input = classify(dir.path()).unwrap();
        assert!(matches!(input, PackInput::Directory(_)));
    }

    #[test]
    fn classify_zip() {
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("payload.zip");
        fs::write(&zip_path, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
        let input = classify(&zip_path).unwrap();
        assert!(matches!(input, PackInput::Zip(_)));
    }

    #[test]
    fn classify_single_html() {
        let dir = tempdir().unwrap();
        let html_path = dir.path().join("page.html");
        fs::write(&html_path, b"<html></html>").unwrap();
        let input = classify(&html_path).unwrap();
        assert!(matches!(input, PackInput::SingleHtml(_)));
    }

    #[test]
    fn classify_rejects_other_single_file() {
        let dir = tempdir().unwrap();
        let txt = dir.path().join("note.txt");
        fs::write(&txt, b"nope").unwrap();
        let err = classify(&txt).unwrap_err();
        match err {
            Error::Other(msg) => assert!(msg.contains("only directories")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn classify_rejects_missing_path() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("ghost");
        assert!(classify(&missing).is_err());
    }

    #[test]
    fn pack_directory_produces_valid_archive() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/HEAD"), b"ref: x").unwrap();
        fs::write(dir.path().join("index.html"), b"<!doctype html>").unwrap();
        fs::write(dir.path().join("style.css"), b"body{}").unwrap();

        let packed = pack(PackInput::Directory(dir.path())).unwrap();
        assert_eq!(packed.file_count, 2);
        assert_eq!(
            packed.uncompressed_size,
            b"<!doctype html>".len() as u64 + b"body{}".len() as u64
        );

        let mut archive = ::zip::ZipArchive::new(Cursor::new(packed.bytes)).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["index.html".to_owned(), "style.css".to_owned()]);
    }

    #[test]
    fn pack_zip_passthrough_preserves_bytes() {
        let dir = tempdir().unwrap();
        let source_dir = tempdir().unwrap();
        fs::write(source_dir.path().join("a.txt"), b"alpha").unwrap();

        // Build a real zip we can pass through.
        let source = pack(PackInput::Directory(source_dir.path())).unwrap();
        let zip_path = dir.path().join("payload.zip");
        fs::write(&zip_path, &source.bytes).unwrap();

        let packed = pack(PackInput::Zip(&zip_path)).unwrap();
        assert_eq!(packed.bytes, source.bytes);
        assert_eq!(packed.file_count, 1);
        assert_eq!(packed.display_filename, "payload.zip");
    }

    #[test]
    fn pack_single_html_wraps_as_stem_index() {
        let dir = tempdir().unwrap();
        let html_path = dir.path().join("report.html");
        fs::write(&html_path, b"<!doctype html><title>R</title>").unwrap();

        let packed = pack(PackInput::SingleHtml(&html_path)).unwrap();
        assert_eq!(packed.file_count, 1);
        assert_eq!(packed.display_filename, "report.zip");

        let mut archive = ::zip::ZipArchive::new(Cursor::new(packed.bytes)).unwrap();
        assert_eq!(archive.len(), 1);
        let mut file = archive.by_index(0).unwrap();
        assert_eq!(file.name(), "report/index.html");
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).unwrap();
        assert_eq!(contents, b"<!doctype html><title>R</title>");
    }

    #[test]
    fn check_archive_size_rejects_above_limit() {
        let err = check_archive_size(MAX_ARCHIVE_BYTES + 1).unwrap_err();
        match err {
            Error::Other(msg) => assert!(msg.contains("archive too large")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn check_archive_size_accepts_at_or_below_limit() {
        check_archive_size(0).expect("zero bytes is fine");
        check_archive_size(MAX_ARCHIVE_BYTES).expect("exact cap is fine");
    }

    #[test]
    fn pack_rejects_zip_passthrough_exceeding_limit() {
        // `pack_zip_passthrough` validates the on-disk file size before
        // any read, so a sparse file with a logical 50MB+ length still
        // trips the bound without allocating real bytes.
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("payload.zip");
        let f = fs::File::create(&zip_path).unwrap();
        f.set_len(MAX_ARCHIVE_BYTES + 1).unwrap();

        let err = pack(PackInput::Zip(&zip_path)).unwrap_err();
        match err {
            Error::Other(msg) => assert!(msg.contains("archive too large")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn pack_zip_rejects_path_traversal_entry() {
        // Build a tiny zip whose only entry is an absolute path. The
        // passthrough must reject it client-side rather than handing the
        // bytes to the upload pipeline.
        use std::io::Write;
        let dir = tempdir().unwrap();
        let zip_path = dir.path().join("bad.zip");
        let mut buf = Cursor::new(Vec::<u8>::new());
        {
            let mut w = ::zip::ZipWriter::new(&mut buf);
            // `..` segments are the canonical zip-slip vector — `enclosed_name`
            // returns `None` for any entry that resolves outside the implicit
            // archive root, regardless of how many parents the entry climbs.
            w.start_file(
                "../etc/passwd",
                ::zip::write::SimpleFileOptions::default()
                    .compression_method(::zip::CompressionMethod::Stored),
            )
            .unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        fs::write(&zip_path, buf.into_inner()).unwrap();

        let err = pack(PackInput::Zip(&zip_path)).unwrap_err();
        match err {
            Error::Other(msg) => assert!(msg.contains("unsafe path"), "msg: {msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
