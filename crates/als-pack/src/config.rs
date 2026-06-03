//! Optional `als.toml` at the root of the path being deployed.
//!
//! v1 schema is intentionally small:
//!
//! ```toml
//! id           = "k7x2qm4j6p"          # any mode: pinned site id
//! name         = "fibonacci-demo"      # any mode: project name (server uniqueness key)
//! title        = "Q3 Fibonacci demo"   # code mode: viewer page title
//! default_file = "src/fibonacci.ts"    # code mode: viewer initial file
//! exclude      = ["scratch/**", "*.log"]  # any mode: extra ignore patterns
//! ```
//!
//! Every field is optional. Unknown fields are rejected at parse time
//! so typos surface immediately instead of being silently ignored.
//!
//! `id` is the authoritative cross-machine pin: when present the CLI
//! sends it as the wire `site_id` on every deploy, so a fresh clone on
//! a different machine resolves to the same site without any local pin
//! file. The deploy command writes the server-assigned id back into
//! `als.toml` automatically after a successful upload, so the field
//! becomes self-populating once any machine has deployed at least once.
//!
//! `exclude` is consumed by [`crate::ignore::build_matcher`] and
//! applies to every deploy mode (plain site, markdown, mdbook, code).
//! `title` and `default_file` are read by `als-code` for its viewer
//! shell and are no-ops on the other paths.
//!
//! `als.toml` itself is dropped from every bundle by the default
//! ignore set — it's CLI metadata, never content.

use std::fs;
use std::path::Path;

use als_core::Error;
use serde::Deserialize;

/// File name the loader looks for at the root of a directory input.
pub const ALS_TOML: &str = "als.toml";

/// Length of a server-assigned site id (Crockford base32 nanoid).
const ID_LEN: usize = 10;

/// User-supplied config sitting at the root of a directory input.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AlsConfig {
    /// Authoritative site id. Same value the server uses to address the
    /// site on `GET /api/sites/:id`. Committed in the repo, so every
    /// machine that has the source tree resolves to the exact same
    /// site — independent of the local pin store and immune to name
    /// renames. 10-char lowercase base32 (`[a-z2-7]{10}`); the loader
    /// rejects anything else with `Error::Config`. Populated
    /// automatically by `als <path>` after a successful deploy.
    #[serde(default)]
    pub id: Option<String>,

    /// Project name. Same identity key the server uses on every
    /// `POST /api/deploy`. Committed in the repo, so a fresh clone on
    /// any machine resolves to the same site without relying on the
    /// per-machine pin store. Kebab-case lowercase (`a-z`, `0-9`,
    /// `-`); validated by the caller before being sent. Overridden
    /// per-invocation by the `--name` CLI flag.
    #[serde(default)]
    pub name: Option<String>,

    /// Page title for the code-mode viewer shell. Defaults to the
    /// directory basename when not set; for non-code deploys this
    /// field is a no-op.
    #[serde(default)]
    pub title: Option<String>,

    /// File path to open by default in the viewer. If set but missing
    /// from the bundled file set, `als-code` errors with
    /// `code_default_file_missing` so the user catches typos.
    #[serde(default)]
    pub default_file: Option<String>,

    /// Additional gitignore-style patterns layered on top of the
    /// default ignore set. Applied to every deploy mode.
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl AlsConfig {
    /// Read `<root>/als.toml` if present. Missing → `Ok(None)`.
    /// Invalid TOML or unknown fields surface as `Error::Config`.
    pub fn load(root: &Path) -> Result<Option<AlsConfig>, Error> {
        let path = root.join(ALS_TOML);
        if !path.is_file() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)
            .map_err(|e| Error::Config(format!("{ALS_TOML}: read failed: {e}")))?;
        let cfg: AlsConfig = toml::from_str(&text)
            .map_err(|e| Error::Config(format!("{ALS_TOML}: invalid TOML: {e}")))?;
        if let Some(id) = cfg.id.as_deref()
            && !is_valid_site_id(id)
        {
            return Err(Error::Config(format!(
                "{ALS_TOML}: `id` must be a 10-char lowercase base32 site id (got {id:?})"
            )));
        }
        Ok(Some(cfg))
    }

    /// Insert or update the `id` key in `<root>/als.toml`, preserving
    /// any other fields, comments, and formatting the user has put in
    /// the file. Creates the file with a minimal commented body when
    /// it does not yet exist.
    ///
    /// Writes go through a sibling `als.toml.tmp` + `rename` so a
    /// crash / SIGKILL / disk-full mid-write cannot truncate or
    /// corrupt the user's hand-edited file (the deploy hot path
    /// invokes this on every success, and `als.toml` is typically
    /// version-controlled — silently losing comments / extra fields
    /// would be a visible regression).
    ///
    /// Returns `Ok(true)` when the file's on-disk bytes actually
    /// changed (so the caller can refresh a content hash); `Ok(false)`
    /// when the id was already correct.
    pub fn write_id(root: &Path, id: &str) -> Result<bool, Error> {
        if !is_valid_site_id(id) {
            return Err(Error::Config(format!(
                "refusing to write malformed id {id:?} to {ALS_TOML}"
            )));
        }
        let path = root.join(ALS_TOML);
        let existing = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(Error::Config(format!("{ALS_TOML}: read failed: {e}"))),
        };

        // Fresh-file path: emit a minimal body with a one-line explainer
        // so a reader stumbling on the file knows where it came from.
        // We bypass toml_edit's render here — there are no existing
        // comments to preserve and we want full control over the seed.
        if existing.is_empty() {
            let body = format!(
                "# Auto-written by `als <path>`; commit me to pin the site.\nid = \"{id}\"\n"
            );
            atomic_write(root, &path, body.as_bytes())?;
            return Ok(true);
        }

        let mut doc = existing
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| Error::Config(format!("{ALS_TOML}: invalid TOML: {e}")))?;

        // No-op fast path: the file already pins the same id.
        if let Some(item) = doc.get("id")
            && item.as_str() == Some(id)
        {
            return Ok(false);
        }

        doc.insert("id", toml_edit::value(id));
        let rendered = doc.to_string();
        if rendered == existing {
            return Ok(false);
        }
        atomic_write(root, &path, rendered.as_bytes())?;
        Ok(true)
    }
}

/// Write `contents` to `final_path` via a sibling `<file>.tmp` + `rename`
/// so a partial write cannot replace the live file. The temp file is
/// removed on any failure path so a stray `.tmp` does not linger.
///
/// `root` is used to anchor the tmp file in the same directory as the
/// target — `rename` across mount points is not atomic, so colocating
/// the tmp file with its destination keeps the rename in-filesystem.
fn atomic_write(root: &Path, final_path: &Path, contents: &[u8]) -> Result<(), Error> {
    let file_name = final_path
        .file_name()
        .ok_or_else(|| Error::Config(format!("{ALS_TOML}: target has no file name")))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = root.join(tmp_name);
    let result = (|| -> Result<(), Error> {
        fs::write(&tmp_path, contents)
            .map_err(|e| Error::Config(format!("{ALS_TOML}: write failed: {e}")))?;
        fs::rename(&tmp_path, final_path)
            .map_err(|e| Error::Config(format!("{ALS_TOML}: rename failed: {e}")))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

fn is_valid_site_id(s: &str) -> bool {
    s.len() == ID_LEN && s.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn missing_file_yields_none() {
        let dir = tempdir().unwrap();
        assert!(AlsConfig::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn empty_file_parses_with_defaults() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(ALS_TOML), b"").unwrap();
        let cfg = AlsConfig::load(dir.path()).unwrap().expect("Some");
        assert!(cfg.id.is_none());
        assert!(cfg.name.is_none());
        assert!(cfg.title.is_none());
        assert!(cfg.default_file.is_none());
        assert!(cfg.exclude.is_empty());
    }

    #[test]
    fn all_fields_parse() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(ALS_TOML),
            br#"
id           = "k7x2qm4j6p"
name         = "q3-demo"
title        = "Q3 Demo"
default_file = "src/main.rs"
exclude      = ["scratch/**", "*.log"]
"#,
        )
        .unwrap();
        let cfg = AlsConfig::load(dir.path()).unwrap().expect("Some");
        assert_eq!(cfg.id.as_deref(), Some("k7x2qm4j6p"));
        assert_eq!(cfg.name.as_deref(), Some("q3-demo"));
        assert_eq!(cfg.title.as_deref(), Some("Q3 Demo"));
        assert_eq!(cfg.default_file.as_deref(), Some("src/main.rs"));
        assert_eq!(
            cfg.exclude,
            vec!["scratch/**".to_owned(), "*.log".to_owned()]
        );
    }

    #[test]
    fn invalid_toml_surfaces_config_error() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(ALS_TOML), b"title = ['unterminated\n").unwrap();
        let err = AlsConfig::load(dir.path()).unwrap_err();
        match err {
            Error::Config(msg) => assert!(msg.contains(ALS_TOML), "msg: {msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn unknown_field_is_rejected() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(ALS_TOML),
            br#"title = "x"
font_family = "Comic Sans"
"#,
        )
        .unwrap();
        let err = AlsConfig::load(dir.path()).unwrap_err();
        match err {
            Error::Config(msg) => assert!(
                msg.contains("font_family") || msg.contains("unknown"),
                "msg: {msg}"
            ),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn malformed_id_is_rejected() {
        let dir = tempdir().unwrap();
        // Uppercase / wrong length / out-of-alphabet → all should fail.
        for bad in ["ABCDEFGHIJ", "tooshort", "0bcdefghij", "abc-efghij"] {
            fs::write(
                dir.path().join(ALS_TOML),
                format!("id = \"{bad}\"\n").as_bytes(),
            )
            .unwrap();
            let err = AlsConfig::load(dir.path()).unwrap_err();
            match err {
                Error::Config(msg) => assert!(
                    msg.contains("id") && msg.contains(bad),
                    "msg: {msg} (input: {bad})"
                ),
                other => panic!("unexpected error: {other:?} (input: {bad})"),
            }
        }
    }

    #[test]
    fn write_id_creates_file_when_missing() {
        let dir = tempdir().unwrap();
        let changed = AlsConfig::write_id(dir.path(), "k7x2qm4j6p").unwrap();
        assert!(changed, "first write must change the on-disk bytes");
        let text = fs::read_to_string(dir.path().join(ALS_TOML)).unwrap();
        assert!(text.contains("id = \"k7x2qm4j6p\""), "body: {text}");
        // Reload should round-trip cleanly.
        let cfg = AlsConfig::load(dir.path()).unwrap().expect("Some");
        assert_eq!(cfg.id.as_deref(), Some("k7x2qm4j6p"));
    }

    #[test]
    fn write_id_is_idempotent_when_already_correct() {
        let dir = tempdir().unwrap();
        AlsConfig::write_id(dir.path(), "k7x2qm4j6p").unwrap();
        let changed = AlsConfig::write_id(dir.path(), "k7x2qm4j6p").unwrap();
        assert!(!changed, "second write with same id must report no change");
    }

    #[test]
    fn write_id_preserves_existing_comments_and_fields() {
        let dir = tempdir().unwrap();
        let original = r#"# Hand-edited preamble — keep me!
name  = "q3-demo"   # uniqueness key
title = "Q3"
exclude = [
    "scratch/**",
]
"#;
        fs::write(dir.path().join(ALS_TOML), original.as_bytes()).unwrap();
        let changed = AlsConfig::write_id(dir.path(), "abcdefghij").unwrap();
        assert!(changed);
        let text = fs::read_to_string(dir.path().join(ALS_TOML)).unwrap();
        assert!(text.contains("Hand-edited preamble"), "body: {text}");
        assert!(text.contains("uniqueness key"), "body: {text}");
        assert!(text.contains("scratch/**"), "body: {text}");
        assert!(text.contains("abcdefghij"), "body: {text}");
        // Reload still parses with every field intact.
        let cfg = AlsConfig::load(dir.path()).unwrap().expect("Some");
        assert_eq!(cfg.id.as_deref(), Some("abcdefghij"));
        assert_eq!(cfg.name.as_deref(), Some("q3-demo"));
        assert_eq!(cfg.title.as_deref(), Some("Q3"));
        assert_eq!(cfg.exclude, vec!["scratch/**".to_owned()]);
    }

    #[test]
    fn write_id_updates_existing_id_in_place() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(ALS_TOML),
            br#"id = "abcdefghij"
name = "q3"
"#,
        )
        .unwrap();
        let changed = AlsConfig::write_id(dir.path(), "k7x2qm4j6p").unwrap();
        assert!(changed);
        let cfg = AlsConfig::load(dir.path()).unwrap().expect("Some");
        assert_eq!(cfg.id.as_deref(), Some("k7x2qm4j6p"));
        assert_eq!(cfg.name.as_deref(), Some("q3"));
    }

    #[test]
    fn write_id_refuses_malformed_value() {
        let dir = tempdir().unwrap();
        let err = AlsConfig::write_id(dir.path(), "NOTVALID").unwrap_err();
        match err {
            Error::Config(msg) => assert!(msg.contains("NOTVALID"), "msg: {msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn write_id_does_not_leave_tmp_file_after_success() {
        // The atomic tmp+rename helper must leave nothing behind on the
        // happy path — a leftover `als.toml.tmp` would surface in `git
        // status` and confuse subsequent loads.
        let dir = tempdir().unwrap();
        AlsConfig::write_id(dir.path(), "k7x2qm4j6p").unwrap();
        assert!(dir.path().join(ALS_TOML).is_file());
        assert!(
            !dir.path().join("als.toml.tmp").exists(),
            "tmp file must be renamed away on success"
        );
        // Re-write to exercise the update branch too.
        AlsConfig::write_id(dir.path(), "abcdefghij").unwrap();
        assert!(!dir.path().join("als.toml.tmp").exists());
    }
}
