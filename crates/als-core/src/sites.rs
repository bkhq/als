//! Per-site local pin store — `~/.config/als/sites/<id>_<name>.toml`.
//!
//! Each successful `als <path>` deploy writes one TOML file under
//! `<config-dir>/als/sites/`. The filename combines the immutable site
//! id with the project name (which the CLI's `--name` validator
//! already restricts to lowercase kebab-case) so a `ls` of the
//! directory is human-scannable AND `starts_with("<id>_")` /
//! `ends_with("_<name>.toml")` lookups stay O(1) in IO terms — we
//! match on filenames before reading any TOML.
//!
//! File contents:
//!
//! ```toml
//! name              = "fox042"            # lowercase kebab-case
//! id                = "k7x2qm4j6p"
//! path              = "/home/alice/build" # canonical absolute path
//! url               = "https://k7x2qm4j6p.a.ls"
//! last_published    = "2026-05-15T19:50:00Z"
//! last_content_hash = "a3f1c2..."         # SHA-256 of post-filter sources
//! ```
//!
//! Lookups:
//!
//! * by id   → filename starts with `<id>_`.
//! * by name → filename ends with `_<name>.toml`.
//! * by path → `load_all` then linear scan on the parsed `path` field.
//!
//! On rename (`als site --name <new>` or a re-deploy with a different
//! `--name`), [`upsert`] removes the stale `<id>_<old>.toml` before
//! writing `<id>_<new>.toml` so the id ↔ file mapping stays 1:1.

use std::path::{Path, PathBuf};

use etcetera::{BaseStrategy, choose_base_strategy};
use serde::{Deserialize, Serialize};

use crate::error::Error;

const APP_DIR: &str = "als";
const SITES_DIR: &str = "sites";
const FILE_EXT: &str = "toml";

/// One pin binding. The CLI's `--name` validator already enforces
/// lowercase, so `name` here is always lowercase in practice; the
/// filename derivation lowercases anyway as a defensive normalisation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteEntry {
    pub name: String,
    pub id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub last_published: String,
    /// SHA-256 fingerprint of the source content at last publish.
    /// Used by `als <path>` (with no metadata flags) to skip a no-op
    /// upload — see `als_pack::digest`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_content_hash: Option<String>,
}

/// XDG-resolved directory holding all pin files.
pub fn sites_dir() -> Result<PathBuf, Error> {
    let strategy = choose_base_strategy()
        .map_err(|e| Error::Config(format!("cannot resolve config directory: {e}")))?;
    Ok(strategy.config_dir().join(APP_DIR).join(SITES_DIR))
}

/// `<id>_<name>.toml` — the canonical pin filename for the given
/// `(id, name)` pair. The name is lowercased defensively in case a
/// hand-edited file uses mixed casing.
fn site_path(dir: &Path, id: &str, name: &str) -> PathBuf {
    let lower = name.to_ascii_lowercase();
    dir.join(format!("{id}_{lower}.{FILE_EXT}"))
}

/// Iterate every `.toml` direntry under `dir`. Missing directory
/// yields an empty vec.
fn read_pin_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(e)),
    };
    let mut out = Vec::new();
    for entry in read {
        let entry = entry.map_err(Error::Io)?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some(FILE_EXT) {
            continue;
        }
        out.push(path);
    }
    Ok(out)
}

/// Filename-based filename lookups. Both `find_by_id_in` and
/// `find_by_name_in` walk the directory once and match on the file
/// stem — no TOML parsing required for the miss case.
fn find_path_by_id(dir: &Path, id: &str) -> Result<Option<PathBuf>, Error> {
    let prefix = format!("{id}_");
    for path in read_pin_files(dir)? {
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with(&prefix) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn find_path_by_name(dir: &Path, name: &str) -> Result<Option<PathBuf>, Error> {
    let suffix = format!("_{}.{}", name.to_ascii_lowercase(), FILE_EXT);
    for path in read_pin_files(dir)? {
        let Some(fname) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if fname.ends_with(&suffix) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn read_entry(path: &Path) -> Result<SiteEntry, Error> {
    let text = std::fs::read_to_string(path)?;
    toml::from_str::<SiteEntry>(&text)
        .map_err(|e| Error::Config(format!("failed to parse {}: {e}", path.display())))
}

/// Load a single pin by site id. Filename-prefix match — no full-dir
/// parse needed unless a file matches.
pub fn load_by_id(id: &str) -> Result<Option<SiteEntry>, Error> {
    let dir = sites_dir()?;
    load_by_id_in(&dir, id)
}

/// Test seam for [`load_by_id`].
pub fn load_by_id_in(dir: &Path, id: &str) -> Result<Option<SiteEntry>, Error> {
    let Some(path) = find_path_by_id(dir, id)? else {
        return Ok(None);
    };
    read_entry(&path).map(Some)
}

/// Load a single pin by project name (case-insensitive).
pub fn load_by_name(name: &str) -> Result<Option<SiteEntry>, Error> {
    let dir = sites_dir()?;
    load_by_name_in(&dir, name)
}

/// Test seam for [`load_by_name`].
pub fn load_by_name_in(dir: &Path, name: &str) -> Result<Option<SiteEntry>, Error> {
    let Some(path) = find_path_by_name(dir, name)? else {
        return Ok(None);
    };
    read_entry(&path).map(Some)
}

/// Load every pin file under [`sites_dir`]. Missing directory or no
/// files → empty vec. Entries are sorted by lowercased name so callers
/// can rely on deterministic iteration.
pub fn load_all() -> Result<Vec<SiteEntry>, Error> {
    let dir = sites_dir()?;
    load_all_in(&dir)
}

/// Test seam for [`load_all`].
pub fn load_all_in(dir: &Path) -> Result<Vec<SiteEntry>, Error> {
    let mut out: Vec<SiteEntry> = Vec::new();
    for path in read_pin_files(dir)? {
        out.push(read_entry(&path)?);
    }
    out.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    Ok(out)
}

/// In-memory find by canonical path. Case-sensitive (filesystems
/// vary; we keep what the OS reported at canonicalisation time).
pub fn find_by_path<'a>(entries: &'a [SiteEntry], canonical: &Path) -> Option<&'a SiteEntry> {
    let needle = canonical.to_string_lossy();
    entries.iter().find(|e| e.path == needle)
}

/// In-memory find by site id. The on-disk [`load_by_id`] is more
/// efficient when you only need one entry; this is for callers that
/// already loaded everything.
pub fn find_by_id<'a>(entries: &'a [SiteEntry], id: &str) -> Option<&'a SiteEntry> {
    entries.iter().find(|e| e.id == id)
}

/// Write (or replace) a pin file. The filename is `<id>_<name>.toml`.
/// If a file already exists for this id under a different name
/// (rename case), it is removed before the new one is written so the
/// id ↔ file mapping stays 1:1.
pub fn upsert(entry: &SiteEntry) -> Result<(), Error> {
    let dir = sites_dir()?;
    upsert_in(&dir, entry)
}

/// Test seam for [`upsert`].
pub fn upsert_in(dir: &Path, entry: &SiteEntry) -> Result<(), Error> {
    std::fs::create_dir_all(dir)?;
    let target = site_path(dir, &entry.id, &entry.name);

    // Rename case: a previous `<id>_<old>.toml` for this id under a
    // different name must go before we write the new file.
    if let Some(existing) = find_path_by_id(dir, &entry.id)?
        && existing != target
    {
        let _ = std::fs::remove_file(&existing);
    }

    let serialized = toml::to_string_pretty(entry)
        .map_err(|e| Error::Config(format!("serialize site entry: {e}")))?;
    let mut tmp_name = target
        .file_name()
        .ok_or_else(|| Error::Config(format!("site path has no file name: {}", target.display())))?
        .to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = dir.join(tmp_name);
    let result = (|| -> Result<(), Error> {
        std::fs::write(&tmp_path, serialized.as_bytes())?;
        std::fs::rename(&tmp_path, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

/// Delete the pin file matching `id`. Returns `Ok(true)` if a file
/// was removed.
pub fn remove_by_id(id: &str) -> Result<bool, Error> {
    let dir = sites_dir()?;
    remove_by_id_in(&dir, id)
}

/// Test seam for [`remove_by_id`].
pub fn remove_by_id_in(dir: &Path, id: &str) -> Result<bool, Error> {
    let Some(path) = find_path_by_id(dir, id)? else {
        return Ok(false);
    };
    std::fs::remove_file(&path)?;
    Ok(true)
}

/// Delete the pin file matching `name` (case-insensitive). Returns
/// `Ok(true)` if a file was removed.
pub fn remove_by_name(name: &str) -> Result<bool, Error> {
    let dir = sites_dir()?;
    remove_by_name_in(&dir, name)
}

/// Test seam for [`remove_by_name`].
pub fn remove_by_name_in(dir: &Path, name: &str) -> Result<bool, Error> {
    let Some(path) = find_path_by_name(dir, name)? else {
        return Ok(false);
    };
    std::fs::remove_file(&path)?;
    Ok(true)
}

/// Drop every pin matching `key`. `key` is matched against the
/// canonical path, project name (case-insensitive), and site id.
pub fn remove_by_key(key: &str) -> Result<Vec<SiteEntry>, Error> {
    let dir = sites_dir()?;
    remove_by_key_in(&dir, key)
}

/// Test seam for [`remove_by_key`].
pub fn remove_by_key_in(dir: &Path, key: &str) -> Result<Vec<SiteEntry>, Error> {
    let all = load_all_in(dir)?;
    let needle_lower = key.to_ascii_lowercase();
    let canonical_match = Path::new(key)
        .canonicalize()
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let mut removed = Vec::new();
    for entry in &all {
        let hit = entry.path == key
            || canonical_match.as_deref() == Some(entry.path.as_str())
            || entry.name.to_ascii_lowercase() == needle_lower
            || entry.id == key;
        if hit {
            remove_by_id_in(dir, &entry.id)?;
            removed.push(entry.clone());
        }
    }
    Ok(removed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample(path: &str, name: &str, id: &str) -> SiteEntry {
        SiteEntry {
            name: name.to_owned(),
            id: id.to_owned(),
            path: path.to_owned(),
            url: Some(format!("https://{id}.a.ls")),
            last_published: "2026-05-15T19:50:00Z".to_owned(),
            last_content_hash: None,
        }
    }

    #[test]
    fn missing_dir_loads_as_empty() {
        let dir = tempdir().unwrap();
        let entries = load_all_in(&dir.path().join("nope")).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn upsert_writes_id_underscore_name_file() {
        let dir = tempdir().unwrap();
        let sites = dir.path().join("sites");
        upsert_in(&sites, &sample("/home/alice/build", "myproj", "k7x2qm4j6p")).unwrap();
        assert!(sites.join("k7x2qm4j6p_myproj.toml").exists());
    }

    #[test]
    fn load_by_id_uses_filename_prefix() {
        let dir = tempdir().unwrap();
        let sites = dir.path().join("sites");
        upsert_in(&sites, &sample("/p", "myproj", "k7x2qm4j6p")).unwrap();
        let loaded = load_by_id_in(&sites, "k7x2qm4j6p").unwrap().expect("hit");
        assert_eq!(loaded.name, "myproj");
        assert!(load_by_id_in(&sites, "nope0000ab").unwrap().is_none());
    }

    #[test]
    fn load_by_name_is_case_insensitive() {
        let dir = tempdir().unwrap();
        let sites = dir.path().join("sites");
        upsert_in(&sites, &sample("/p", "myproj", "k7x2qm4j6p")).unwrap();
        assert!(load_by_name_in(&sites, "MYPROJ").unwrap().is_some());
        assert!(load_by_name_in(&sites, "myproj").unwrap().is_some());
        assert!(load_by_name_in(&sites, "other").unwrap().is_none());
    }

    #[test]
    fn upsert_under_rename_drops_stale_file() {
        let dir = tempdir().unwrap();
        let sites = dir.path().join("sites");
        upsert_in(&sites, &sample("/p", "oldname", "k7x2qm4j6p")).unwrap();
        assert!(sites.join("k7x2qm4j6p_oldname.toml").exists());

        // Same id, different name (the rename case).
        upsert_in(&sites, &sample("/p", "newname", "k7x2qm4j6p")).unwrap();
        assert!(!sites.join("k7x2qm4j6p_oldname.toml").exists());
        assert!(sites.join("k7x2qm4j6p_newname.toml").exists());
        assert_eq!(load_all_in(&sites).unwrap().len(), 1);
    }

    #[test]
    fn find_by_path_returns_entry() {
        let dir = tempdir().unwrap();
        let sites = dir.path().join("sites");
        upsert_in(&sites, &sample("/home/alice/build", "myproj", "k7x2qm4j6p")).unwrap();
        let entries = load_all_in(&sites).unwrap();
        let hit = find_by_path(&entries, Path::new("/home/alice/build")).expect("hit");
        assert_eq!(hit.name, "myproj");
        assert!(find_by_path(&entries, Path::new("/home/alice/other")).is_none());
    }

    #[test]
    fn remove_by_id_returns_false_for_missing() {
        let dir = tempdir().unwrap();
        let sites = dir.path().join("sites");
        assert!(!remove_by_id_in(&sites, "ghost00000").unwrap());
    }

    #[test]
    fn remove_by_key_matches_path_name_or_id() {
        let dir = tempdir().unwrap();
        let sites = dir.path().join("sites");
        upsert_in(&sites, &sample("/a", "alpha", "k7x2qm4j6p")).unwrap();
        upsert_in(&sites, &sample("/b", "beta", "m3p4rs5t2k")).unwrap();
        upsert_in(&sites, &sample("/c", "gamma", "q4f2at3v7y")).unwrap();

        let dropped = remove_by_key_in(&sites, "beta").unwrap();
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].id, "m3p4rs5t2k");
        assert_eq!(load_all_in(&sites).unwrap().len(), 2);

        let dropped = remove_by_key_in(&sites, "q4f2at3v7y").unwrap();
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].name, "gamma");
        assert_eq!(load_all_in(&sites).unwrap().len(), 1);

        let dropped = remove_by_key_in(&sites, "nope").unwrap();
        assert!(dropped.is_empty());
        assert_eq!(load_all_in(&sites).unwrap().len(), 1);
    }
}
