//! `manifest.json` shape consumed by `toss-code-viewer`.
//!
//! Schema:
//!
//! ```json
//! {
//!   "schemaVersion": 1,
//!   "title": "fibonacci",
//!   "files": [
//!     { "path": "src/fibonacci.ts", "size": 1234, "lang": "typescript" }
//!   ],
//!   "defaultFile": "src/fibonacci.ts"
//! }
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::lang::lang_for;
use crate::walker::Entry;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub title: String,
    pub files: Vec<FileEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub lang: String,
}

/// Build a manifest from walker output plus a title.
///
/// `forced_default` is used by single-file inputs: the lone file is
/// always the default regardless of name pattern.
pub(crate) fn build(title: &str, entries: &[Entry], forced_default: Option<&str>) -> Manifest {
    let mut files: Vec<FileEntry> = entries
        .iter()
        .map(|e| FileEntry {
            path: e.rel_path.to_string_lossy().into_owned(),
            size: e.size,
            lang: lang_for(&e.rel_path).to_owned(),
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let default_file = forced_default
        .map(str::to_owned)
        .or_else(|| pick_default(&files).map(str::to_owned));
    Manifest {
        schema_version: SCHEMA_VERSION,
        title: title.to_owned(),
        files,
        default_file,
    }
}

/// Default-file selection:
///
/// 1. First top-level entry matching `^(main|index|app)\.\w+$`.
/// 2. Otherwise the alphabetically first file.
fn pick_default(files: &[FileEntry]) -> Option<&str> {
    let preferred = files
        .iter()
        .find(|f| {
            let Some(stem) = Path::new(&f.path).file_stem().and_then(|s| s.to_str()) else {
                return false;
            };
            // Top-level only — no `/` in path.
            if f.path.contains('/') {
                return false;
            }
            matches!(stem.to_ascii_lowercase().as_str(), "main" | "index" | "app")
        })
        .map(|f| f.path.as_str());
    preferred.or_else(|| files.first().map(|f| f.path.as_str()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn entry(path: &str, size: u64) -> Entry {
        Entry {
            abs_path: PathBuf::from(format!("/tmp/{path}")),
            rel_path: PathBuf::from(path),
            size,
        }
    }

    #[test]
    fn schema_fields_serialize_camel_case() {
        let m = build("demo", &[entry("a.ts", 1)], None);
        let json = serde_json::to_value(&m).unwrap();
        assert!(json.get("schemaVersion").is_some());
        assert!(json.get("defaultFile").is_some());
        assert!(json.get("files").is_some());
    }

    #[test]
    fn forced_default_wins() {
        let entries = [entry("a.ts", 1), entry("main.rs", 1)];
        let m = build("t", &entries, Some("a.ts"));
        assert_eq!(m.default_file.as_deref(), Some("a.ts"));
    }

    #[test]
    fn main_at_root_wins_over_alphabetical() {
        let entries = [
            entry("aardvark.rs", 1),
            entry("main.rs", 1),
            entry("zebra.rs", 1),
        ];
        let m = build("t", &entries, None);
        assert_eq!(m.default_file.as_deref(), Some("main.rs"));
    }

    #[test]
    fn index_at_root_wins_over_alphabetical() {
        let entries = [entry("aardvark.ts", 1), entry("index.ts", 1)];
        let m = build("t", &entries, None);
        assert_eq!(m.default_file.as_deref(), Some("index.ts"));
    }

    #[test]
    fn app_at_root_wins_over_alphabetical() {
        let entries = [entry("aardvark.js", 1), entry("app.js", 1)];
        let m = build("t", &entries, None);
        assert_eq!(m.default_file.as_deref(), Some("app.js"));
    }

    #[test]
    fn nested_main_does_not_count_as_default() {
        let entries = [entry("aardvark.ts", 1), entry("src/main.ts", 1)];
        let m = build("t", &entries, None);
        // Nested `main` does *not* win — falls through to alphabetical.
        assert_eq!(m.default_file.as_deref(), Some("aardvark.ts"));
    }

    #[test]
    fn alphabetical_fallback_used_otherwise() {
        let entries = [entry("zebra.rs", 1), entry("aardvark.rs", 1)];
        let m = build("t", &entries, None);
        assert_eq!(m.default_file.as_deref(), Some("aardvark.rs"));
    }

    #[test]
    fn empty_entries_yield_no_default() {
        let m = build("t", &[], None);
        assert!(m.default_file.is_none());
        assert!(m.files.is_empty());
    }

    #[test]
    fn lang_inferred_from_extension() {
        let entries = [entry("foo.rs", 1), entry("foo.unknown", 1)];
        let m = build("t", &entries, None);
        let langs: Vec<&str> = m.files.iter().map(|f| f.lang.as_str()).collect();
        assert!(langs.contains(&"rust"));
        assert!(langs.contains(&"text"));
    }
}
