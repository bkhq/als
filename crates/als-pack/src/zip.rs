//! In-memory zip writer used by the `pack` entry point.
//!
//! Deflate compression only (pure-Rust `flate2`); other methods are not
//! enabled in the workspace pin. Archive entry paths are validated against
//! traversal escapes (`..`, absolute roots) defensively even though
//! `walk::walk` already produces relative paths under the walk root.

use std::io::Cursor;
use std::path::Path;

use als_core::Error;

use crate::walk::Entry;

/// Build a zip archive in memory from a stream of `Entry` records.
pub fn write_zip(entries: impl IntoIterator<Item = Entry>) -> Result<Vec<u8>, Error> {
    let buf = Vec::with_capacity(1024 * 1024);
    let mut writer = ::zip::ZipWriter::new(Cursor::new(buf));
    let options = ::zip::write::SimpleFileOptions::default()
        .compression_method(::zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    for entry in entries {
        let rel = entry.rel_path.to_string_lossy();
        validate_archive_path(rel.as_ref(), &entry.rel_path)?;
        writer
            .start_file(rel.as_ref(), options)
            .map_err(|e| Error::Other(format!("zip start_file '{rel}': {e}")))?;
        let mut file = std::fs::File::open(&entry.abs_path)?;
        std::io::copy(&mut file, &mut writer)?;
    }

    let cursor = writer
        .finish()
        .map_err(|e| Error::Other(format!("zip finish: {e}")))?;
    Ok(cursor.into_inner())
}

fn validate_archive_path(rel: &str, path: &Path) -> Result<(), Error> {
    if rel.is_empty() {
        return Err(Error::Other("empty archive entry path".into()));
    }
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(Error::Other(format!("absolute archive entry path: {rel}")));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(Error::Other(format!("unsafe archive entry path: {rel}")));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::fs;
    use std::io::Read;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn writes_files_with_forward_slash_paths() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("hello.txt");
        fs::write(&file_path, b"hi").unwrap();

        let entries = vec![Entry {
            abs_path: file_path,
            rel_path: PathBuf::from("nested/hello.txt"),
            size: 2,
        }];
        let bytes = write_zip(entries).unwrap();

        let mut archive = ::zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 1);
        let mut file = archive.by_index(0).unwrap();
        assert_eq!(file.name(), "nested/hello.txt");
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).unwrap();
        assert_eq!(contents, b"hi");
    }

    #[test]
    fn rejects_parent_traversal() {
        let result = validate_archive_path("../etc/passwd", &PathBuf::from("../etc/passwd"));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_absolute_path() {
        let result = validate_archive_path("/etc/passwd", &PathBuf::from("/etc/passwd"));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_empty_path() {
        let result = validate_archive_path("", &PathBuf::from(""));
        assert!(result.is_err());
    }
}
