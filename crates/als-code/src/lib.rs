//! Bundle a path of code into a self-contained upload for the
//! `toss-code-viewer` `CodeMirror` viewer.
//!
//! Used by the als binary for the `code` upload mode (`als
//! <code-path>` with no `index.html` / `*.md`). The server keeps the
//! same multipart `/api/deploy` contract; the only thing that changes
//! is the shape of the zip the binary builds before upload.
//!
//! Public flow:
//!
//! 1. [`classify`] inspects a path and reports whether it is
//!    code-shaped (a single code file, or a directory of code).
//! 2. [`bundle()`] consumes the classified input and produces a
//!    [`Bundle`] — an owning temp directory containing
//!    `index.html` + `manifest.json` + `raw/*` ready to feed to
//!    `als_pack::pack(PackInput::Directory(bundle.path()))`.
//!
//! Callers pick a [`ViewerSource`] when bundling. Uploads use
//! [`ViewerSource::Cdn`] so the rendered HTML points at jsdelivr at a
//! pinned version; `als preview` uses [`ViewerSource::LocalEmbedded`]
//! to drop a copy of `viewer.js` / `viewer.css` (compiled into the
//! binary from `viewer/` at build time) next to `index.html`, so the
//! preview works offline and tracks the same source the next
//! `npm publish` will produce.

mod bundle;
mod detect;
mod lang;
mod manifest;
mod template;
mod walker;

pub use als_pack::{ALS_TOML, AlsConfig};
pub use bundle::{Bundle, bundle};
pub use detect::{CodeInput, classify, force_input};
pub use lang::lang_for;
pub use manifest::{FileEntry, Manifest};

/// Where the rendered `index.html` should load `viewer.js` / `viewer.css` from.
#[derive(Debug, Clone, Copy)]
pub enum ViewerSource {
    /// Pinned npm CDN URL. Used by production uploads.
    Cdn,
    /// Bundle the local copy compiled into the als binary from the
    /// repo's `viewer/` directory. Used by `als preview` so preview
    /// works without internet access and renders identically to what
    /// the matching CDN version would.
    LocalEmbedded,
}

/// npm package name for the viewer. Every published code site references
/// this via `https://cdn.jsdelivr.net/npm/<VIEWER_PKG>@<VIEWER_VERSION>/viewer.js`.
pub const VIEWER_PKG: &str = "toss-code-viewer";

/// Pinned viewer version. Must be kept in lockstep with `viewer/package.json`.
pub const VIEWER_VERSION: &str = "0.1.0";

/// Default CDN base; jsdelivr because it has aggressive caching and a
/// fallback to the npm registry under the same path scheme.
pub const VIEWER_CDN_BASE: &str = "https://cdn.jsdelivr.net/npm";

/// Bundled `viewer.js` compiled into the binary at build time. The
/// source of truth lives in the sibling `../viewer/` project, which
/// is the npm publish source and the place where `bun run build`
/// produces the bundle. The committed snapshot under
/// `crates/als-code/assets/` is the one consumed here, so building
/// the Rust workspace never requires the sibling project to exist —
/// only releasing a new viewer version does.
///
/// A preview rendered with [`ViewerSource::LocalEmbedded`] therefore
/// matches the published CDN copy as long as the snapshot and the
/// CDN release are kept in lockstep (the `just sync-viewer` recipe
/// + the `VIEWER_VERSION` constant bump enforce that).
pub const EMBEDDED_VIEWER_JS: &str = include_str!("../assets/viewer.js");

/// `viewer.css` counterpart of [`EMBEDDED_VIEWER_JS`].
pub const EMBEDDED_VIEWER_CSS: &str = include_str!("../assets/viewer.css");

/// Maximum number of files included in a code bundle. Larger inputs
/// error with `code_too_many_files`.
pub const MAX_FILES: usize = 50;

/// Per-file byte cap. Larger files error with `code_file_too_large`.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Aggregate byte cap. Sum of included files; error
/// `code_total_too_large` when exceeded.
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024;

/// Maximum directory depth from the root (root is depth 0). Files
/// deeper than this error with `code_depth_exceeded`.
pub const MAX_DEPTH: usize = 8;

/// Compose the CDN URL the rendered HTML shell references for `viewer.js`.
pub fn viewer_cdn_url() -> String {
    format!("{VIEWER_CDN_BASE}/{VIEWER_PKG}@{VIEWER_VERSION}/viewer.js")
}

/// Compose the CDN URL the rendered HTML shell references for `viewer.css`.
pub fn viewer_css_cdn_url() -> String {
    format!("{VIEWER_CDN_BASE}/{VIEWER_PKG}@{VIEWER_VERSION}/viewer.css")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_cdn_url_uses_pinned_version() {
        let url = viewer_cdn_url();
        assert!(url.contains(VIEWER_PKG));
        assert!(url.contains(VIEWER_VERSION));
        assert!(url.ends_with("/viewer.js"));
    }

    #[test]
    fn embedded_viewer_assets_are_non_empty() {
        assert!(EMBEDDED_VIEWER_JS.len() > 1000, "viewer.js looks too small");
        assert!(
            EMBEDDED_VIEWER_CSS.len() > 200,
            "viewer.css looks too small"
        );
        // Sanity: viewer.js must reach for CodeMirror imports so we know
        // the embedded blob actually carries the real source.
        assert!(EMBEDDED_VIEWER_JS.contains("@codemirror/"));
    }
}
