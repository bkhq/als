//! Assemble a code bundle in a tempdir.
//!
//! Output layout when [`ViewerSource::Cdn`] is selected:
//!
//! ```text
//! <tempdir>/
//! ├── index.html        references cdn.jsdelivr.net/<pkg>@<v>/viewer.{js,css}
//! ├── manifest.json
//! └── raw/
//!     ├── README.txt
//!     ├── src/
//!     │   └── main.rs
//!     └── Cargo.toml
//! ```
//!
//! Layout when [`ViewerSource::LocalEmbedded`] is selected — extra
//! `viewer.js` / `viewer.css` sit next to `index.html` and are
//! referenced relatively, so the bundle is fully self-contained and
//! works offline:
//!
//! ```text
//! <tempdir>/
//! ├── index.html        references ./viewer.{js,css}
//! ├── manifest.json
//! ├── viewer.js         compiled-in copy from repo's viewer/viewer.js
//! ├── viewer.css        compiled-in copy from repo's viewer/viewer.css
//! └── raw/
//!     └── ...
//! ```
//!
//! [`bundle`] returns an owning [`Bundle`] handle; the binary then
//! hands `bundle.path()` to `als_pack::pack(PackInput::Directory)`
//! (uploads) or to `als_preview::bind` (preview).

use std::fs;
use std::path::{Path, PathBuf};

use als_core::Error;
use tempfile::TempDir;

use als_pack::AlsConfig;

use crate::ViewerSource;
use crate::detect::{CodeInput, validate_limits};
use crate::manifest::build as build_manifest;
use crate::template;
use crate::walker::{self, Entry};

/// Owning handle for a built code bundle. Dropping cleans up the
/// underlying temp directory.
#[derive(Debug)]
pub struct Bundle {
    dir: TempDir,
    stem: String,
}

impl Bundle {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn stem(&self) -> &str {
        &self.stem
    }
}

/// Materialise a code bundle for `input`, with the viewer assets
/// referenced as `viewer` requests.
pub fn bundle(input: CodeInput<'_>, viewer: ViewerSource) -> Result<Bundle, Error> {
    match input {
        CodeInput::SingleFile(path) => bundle_single(path, viewer),
        CodeInput::Directory(root) => bundle_directory(root, viewer),
    }
}

fn bundle_directory(root: &Path, viewer: ViewerSource) -> Result<Bundle, Error> {
    // `exclude` patterns are layered in by `als-pack::build_matcher`,
    // so the code walker no longer threads them through; here we only
    // need `title` and `default_file`.
    let config = AlsConfig::load(root)?;
    let entries = walker::walk(root)?;
    validate_limits(&entries)?;

    let title = config
        .as_ref()
        .and_then(|c| c.title.clone())
        .unwrap_or_else(|| directory_title(root));

    let forced_default = match config.as_ref().and_then(|c| c.default_file.as_deref()) {
        Some(requested) => {
            ensure_default_exists(requested, &entries)?;
            Some(requested.to_owned())
        }
        None => None,
    };

    let out =
        tempfile::tempdir().map_err(|e| Error::Other(format!("create bundle tempdir: {e}")))?;
    write_bundle(
        out.path(),
        &title,
        &entries,
        forced_default.as_deref(),
        viewer,
    )?;
    Ok(Bundle {
        dir: out,
        stem: title,
    })
}

fn bundle_single(file: &Path, viewer: ViewerSource) -> Result<Bundle, Error> {
    let entries = walker::single(file)?;
    validate_limits(&entries)?;
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("snippet")
        .to_owned();
    let title = file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&stem)
        .to_owned();
    let default = entries
        .first()
        .map(|e| e.rel_path.to_string_lossy().into_owned());
    let out =
        tempfile::tempdir().map_err(|e| Error::Other(format!("create bundle tempdir: {e}")))?;
    write_bundle(out.path(), &title, &entries, default.as_deref(), viewer)?;
    Ok(Bundle { dir: out, stem })
}

fn write_bundle(
    out: &Path,
    title: &str,
    entries: &[Entry],
    forced_default: Option<&str>,
    viewer: ViewerSource,
) -> Result<(), Error> {
    let manifest = build_manifest(title, entries, forced_default);
    let html = template::render(title, &manifest.files, viewer);

    fs::write(out.join("index.html"), html.as_bytes())?;

    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| Error::Other(format!("serialize manifest: {e}")))?;
    fs::write(out.join("manifest.json"), &manifest_json)?;

    if matches!(viewer, ViewerSource::LocalEmbedded) {
        fs::write(out.join("viewer.js"), crate::EMBEDDED_VIEWER_JS.as_bytes())?;
        fs::write(
            out.join("viewer.css"),
            crate::EMBEDDED_VIEWER_CSS.as_bytes(),
        )?;
    }

    let raw_root = out.join("raw");
    fs::create_dir(&raw_root)?;
    for entry in entries {
        let dest = raw_root.join(&entry.rel_path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&entry.abs_path, &dest)
            .map_err(|e| Error::Io(io_with_context(&entry.abs_path, &e)))?;
    }
    Ok(())
}

fn directory_title(root: &Path) -> String {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root));
    canonical
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("code")
        .to_owned()
}

fn ensure_default_exists(requested: &str, entries: &[Entry]) -> Result<(), Error> {
    let normalised = requested.replace('\\', "/");
    let hit = entries
        .iter()
        .any(|e| e.rel_path.to_string_lossy() == normalised);
    if hit {
        return Ok(());
    }
    Err(Error::Other(format!(
        "code_default_file_missing: '{requested}' is not in the bundled file set"
    )))
}

fn io_with_context(path: &Path, err: &std::io::Error) -> std::io::Error {
    std::io::Error::new(
        err.kind(),
        format!("copy '{}' into bundle: {err}", path.display()),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::manifest::Manifest;

    fn read_manifest(path: &Path) -> Manifest {
        let bytes = fs::read(path).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn cdn_bundle(input: CodeInput<'_>) -> Bundle {
        bundle(input, ViewerSource::Cdn).unwrap()
    }

    #[test]
    fn bundle_single_file_layout() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("snippet.ts");
        fs::write(&file, b"export const x = 1;\n").unwrap();

        let b = cdn_bundle(CodeInput::SingleFile(&file));
        assert_eq!(b.stem(), "snippet");

        assert!(b.path().join("index.html").is_file());
        assert!(b.path().join("manifest.json").is_file());
        let raw = b.path().join("raw/snippet.ts");
        assert!(raw.is_file());

        let manifest = read_manifest(&b.path().join("manifest.json"));
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].path, "snippet.ts");
        assert_eq!(manifest.files[0].lang, "typescript");
        assert_eq!(manifest.default_file.as_deref(), Some("snippet.ts"));
    }

    #[test]
    fn bundle_directory_layout() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), b"[package]\n").unwrap();
        fs::write(dir.path().join("src/main.rs"), b"fn main() {}\n").unwrap();

        let b = cdn_bundle(CodeInput::Directory(dir.path()));

        assert!(b.path().join("index.html").is_file());
        assert!(b.path().join("manifest.json").is_file());
        assert!(b.path().join("raw/Cargo.toml").is_file());
        assert!(b.path().join("raw/src/main.rs").is_file());

        let manifest = read_manifest(&b.path().join("manifest.json"));
        let paths: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains("Cargo.toml"));
        assert!(paths.contains("src/main.rs"));
        assert_eq!(manifest.default_file.as_deref(), Some("Cargo.toml"));
    }

    #[test]
    fn bundle_cdn_html_references_cdn_url_and_no_local_files() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.rs");
        fs::write(&file, b"fn main() {}\n").unwrap();
        let b = cdn_bundle(CodeInput::SingleFile(&file));
        let html = fs::read_to_string(b.path().join("index.html")).unwrap();
        assert!(html.contains(&crate::viewer_cdn_url()));
        assert!(html.contains("toss-code-viewer@"));
        assert!(
            !b.path().join("viewer.js").exists(),
            "CDN mode must not write viewer.js into the bundle",
        );
        assert!(!b.path().join("viewer.css").exists());
    }

    #[test]
    fn bundle_local_embedded_writes_viewer_files() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.rs");
        fs::write(&file, b"fn main() {}\n").unwrap();
        let b = bundle(CodeInput::SingleFile(&file), ViewerSource::LocalEmbedded).unwrap();
        assert!(b.path().join("viewer.js").is_file());
        assert!(b.path().join("viewer.css").is_file());
        let html = fs::read_to_string(b.path().join("index.html")).unwrap();
        assert!(html.contains("src=\"./viewer.js\""));
        assert!(html.contains("href=\"./viewer.css\""));
        assert!(
            !html.contains("cdn.jsdelivr.net"),
            "local embedded must not reference any CDN: {html}",
        );
        // The embedded copy must match the compiled-in source byte-for-byte
        // so a preview rendered today renders the same way a publish-now
        // would when the CDN ships the matching version.
        let on_disk = fs::read_to_string(b.path().join("viewer.js")).unwrap();
        assert_eq!(on_disk, crate::EMBEDDED_VIEWER_JS);
    }

    #[test]
    fn bundle_errors_when_empty() {
        let dir = tempdir().unwrap();
        let err = bundle(CodeInput::Directory(dir.path()), ViewerSource::Cdn).unwrap_err();
        match err {
            Error::Other(msg) => assert!(msg.starts_with("code_no_files"), "got: {msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bundle_errors_when_too_many_files() {
        let dir = tempdir().unwrap();
        for i in 0..=crate::MAX_FILES {
            fs::write(dir.path().join(format!("f{i}.rs")), b"//\n").unwrap();
        }
        let err = bundle(CodeInput::Directory(dir.path()), ViewerSource::Cdn).unwrap_err();
        match err {
            Error::Other(msg) => assert!(msg.starts_with("code_too_many_files"), "got: {msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bundle_errors_when_total_too_large() {
        let dir = tempdir().unwrap();
        let chunk_size = usize::try_from(crate::MAX_FILE_BYTES - 1).unwrap();
        let chunk = vec![b'x'; chunk_size];
        let per_file = chunk.len() as u64;
        let needed = usize::try_from(crate::MAX_TOTAL_BYTES / per_file).unwrap() + 2;
        let count = needed.min(crate::MAX_FILES);
        for i in 0..count {
            fs::write(dir.path().join(format!("f{i}.rs")), &chunk).unwrap();
        }
        let err = bundle(CodeInput::Directory(dir.path()), ViewerSource::Cdn).unwrap_err();
        match err {
            Error::Other(msg) => {
                assert!(msg.starts_with("code_total_too_large"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn als_toml_title_overrides_directory_basename() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), b"fn main() {}\n").unwrap();
        fs::write(dir.path().join("als.toml"), br#"title = "Q3 Demo""#).unwrap();

        let b = cdn_bundle(CodeInput::Directory(dir.path()));
        let manifest = read_manifest(&b.path().join("manifest.json"));
        assert_eq!(manifest.title, "Q3 Demo");
        let html = fs::read_to_string(b.path().join("index.html")).unwrap();
        assert!(html.contains("<title>Q3 Demo</title>"));
    }

    #[test]
    fn als_toml_default_file_overrides_alphabetical_fallback() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("aardvark.rs"), b"//\n").unwrap();
        fs::write(dir.path().join("src/main.rs"), b"fn main() {}\n").unwrap();
        fs::write(
            dir.path().join("als.toml"),
            br#"default_file = "src/main.rs""#,
        )
        .unwrap();

        let b = cdn_bundle(CodeInput::Directory(dir.path()));
        let manifest = read_manifest(&b.path().join("manifest.json"));
        assert_eq!(manifest.default_file.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn als_toml_missing_default_file_errors() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), b"fn main() {}\n").unwrap();
        fs::write(
            dir.path().join("als.toml"),
            br#"default_file = "src/oops.rs""#,
        )
        .unwrap();

        let err = bundle(CodeInput::Directory(dir.path()), ViewerSource::Cdn).unwrap_err();
        match err {
            Error::Other(msg) => {
                assert!(msg.starts_with("code_default_file_missing"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn als_toml_exclude_drops_matching_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), b"fn main() {}\n").unwrap();
        fs::write(dir.path().join("scratch.rs"), b"// scratch\n").unwrap();
        fs::write(dir.path().join("als.toml"), br#"exclude = ["scratch.rs"]"#).unwrap();

        let b = cdn_bundle(CodeInput::Directory(dir.path()));
        let manifest = read_manifest(&b.path().join("manifest.json"));
        let paths: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains("main.rs"));
        assert!(!paths.contains("scratch.rs"));
        assert!(!paths.contains("als.toml"));
    }
}
