//! Default ignore patterns and `als.toml.exclude` loader.
//!
//! The matcher seeds an `ignore::gitignore::GitignoreBuilder` with the
//! project default ignore set (including `als.toml` itself), then
//! layers any `exclude = [...]` entries from `<root>/als.toml` on top.
//! Negations in `als.toml.exclude` therefore override the defaults
//! (standard gitignore semantics).
//!
//! `als.toml` is the single source of truth for per-project filtering;
//! there is no separate ignore file.

use std::path::Path;

use als_core::Error;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::config::AlsConfig;

/// Patterns always excluded from packaged archives. Mirrors the
/// "Default ignore" section of `docs/cli-spec.md`. `als.toml` is in
/// the set because it is CLI metadata, never content.
pub const DEFAULT_IGNORES: &[&str] = &[
    ".git/",
    ".svn/",
    ".hg/",
    "node_modules/",
    "target/",
    "dist/",
    "__pycache__/",
    ".DS_Store",
    "Thumbs.db",
    ".env",
    "*.pem",
    "*.key",
    "als.toml",
];

/// Build a `Gitignore` matcher anchored at `root`, seeded with
/// `DEFAULT_IGNORES` and overlaid with the `exclude` patterns read
/// from `<root>/als.toml` if present.
pub fn build_matcher(root: &Path) -> Result<Gitignore, Error> {
    let mut builder = GitignoreBuilder::new(root);
    for pattern in DEFAULT_IGNORES {
        builder
            .add_line(None, pattern)
            .map_err(|e| Error::Other(format!("invalid default ignore '{pattern}': {e}")))?;
    }
    if let Some(cfg) = AlsConfig::load(root)? {
        for pattern in &cfg.exclude {
            builder
                .add_line(None, pattern)
                .map_err(|e| Error::Config(format!("als.toml exclude '{pattern}': {e}")))?;
        }
    }
    builder
        .build()
        .map_err(|e| Error::Other(format!("gitignore build: {e}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn defaults_match_common_patterns() {
        let dir = tempdir().unwrap();
        let matcher = build_matcher(dir.path()).unwrap();

        assert!(matcher.matched(".git", true).is_ignore());
        assert!(matcher.matched("node_modules", true).is_ignore());
        assert!(matcher.matched("target", true).is_ignore());
        assert!(matcher.matched(".env", false).is_ignore());
        assert!(matcher.matched("server.pem", false).is_ignore());
        assert!(matcher.matched("id_rsa.key", false).is_ignore());
        assert!(matcher.matched(".DS_Store", false).is_ignore());
        assert!(
            matcher.matched("als.toml", false).is_ignore(),
            "als.toml itself must be excluded from bundles by default"
        );
    }

    #[test]
    fn als_toml_exclude_layers_on_top() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("als.toml"),
            br#"exclude = ["secrets.txt", "*.log"]"#,
        )
        .unwrap();
        let matcher = build_matcher(dir.path()).unwrap();

        assert!(matcher.matched("secrets.txt", false).is_ignore());
        assert!(matcher.matched("debug.log", false).is_ignore());
        assert!(matcher.matched("README.md", false).is_none());
    }

    #[test]
    fn als_toml_negation_overrides_defaults() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("als.toml"), br#"exclude = ["!.env"]"#).unwrap();
        let matcher = build_matcher(dir.path()).unwrap();

        assert!(matcher.matched(".env", false).is_whitelist());
    }

    #[test]
    fn malformed_als_toml_surfaces_config_error() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("als.toml"), b"exclude = [unterminated\n").unwrap();
        let err = build_matcher(dir.path()).unwrap_err();
        match err {
            Error::Config(msg) => assert!(msg.contains("als.toml"), "msg: {msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
