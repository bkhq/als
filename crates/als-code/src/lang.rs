//! Extension → `CodeMirror` language id mapping.
//!
//! The viewer (`toss-code-viewer/viewer.js`) keys its language loader
//! table by these strings. Unrecognised extensions fall back to
//! `"text"`, which the viewer renders as a plain document without a
//! language pack import.

use std::path::Path;

/// Pairs of `(lowercase extension, viewer language id)`.
///
/// Kept as a static slice (not a hashmap) so it shows up in compiled
/// docs and so adding entries is a one-line patch. Both `.tsx` and
/// `.jsx` deliberately map onto `typescript` / `javascript`
/// respectively to inherit the JSX-aware parser configurations baked
/// into the viewer's loader.
pub(crate) const EXT_TO_LANG: &[(&str, &str)] = &[
    // Web — JS/TS family
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("jsx", "javascript"),
    ("ts", "typescript"),
    ("tsx", "typescript"),
    ("vue", "vue"),
    // Web — markup / style
    ("html", "html"),
    ("htm", "html"),
    ("xml", "xml"),
    ("svg", "xml"),
    ("css", "css"),
    ("scss", "css"),
    ("sass", "css"),
    ("less", "css"),
    // Systems / native
    ("rs", "rust"),
    ("go", "go"),
    ("c", "cpp"),
    ("h", "cpp"),
    ("cc", "cpp"),
    ("cpp", "cpp"),
    ("cxx", "cpp"),
    ("hpp", "cpp"),
    ("hxx", "cpp"),
    ("java", "java"),
    // Scripting
    ("py", "python"),
    ("rb", "javascript"), // CM6 has no ruby lang pack — fall back to JS highlighter
    ("php", "php"),
    ("sh", "javascript"), // shell falls back to JS-ish; viewer uses a plain renderer
    ("bash", "javascript"),
    ("zsh", "javascript"),
    // Data / config
    ("json", "json"),
    ("jsonc", "json"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("toml", "text"),
    ("ini", "text"),
    ("conf", "text"),
    ("env", "text"),
    // Query
    ("sql", "sql"),
    // Markup (rare in code mode since *.md routes to mdbook, but keep
    // the mapping for `--kind code` overrides)
    ("md", "markdown"),
    ("markdown", "markdown"),
    // Plain text
    ("txt", "text"),
    ("log", "text"),
];

/// Look up a language id for `path`'s extension. Returns `"text"` when
/// the extension is missing or unknown.
pub fn lang_for(path: &Path) -> &'static str {
    let Some(ext) = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return "text";
    };
    EXT_TO_LANG
        .iter()
        .find(|(e, _)| *e == ext)
        .map_or("text", |(_, lang)| *lang)
}

/// Strictly "code"-shaped extensions used by [`is_recognized_code`].
///
/// Narrower than [`EXT_TO_LANG`] — config / text formats like `.toml`
/// or `.txt` may end up *inside* a code bundle alongside real source,
/// but they should not on their own trigger code-mode routing
/// (otherwise `als note.txt` would suddenly stop being an error and
/// become a single-file code bundle, breaking existing behaviour).
const CODE_EXTENSIONS: &[&str] = &[
    "js", "mjs", "cjs", "jsx", "ts", "tsx", "vue", "rs", "go", "c", "h", "cc", "cpp", "cxx", "hpp",
    "hxx", "java", "py", "rb", "php", "sh", "bash", "zsh", "sql",
];

/// Return `true` when `path`'s extension is a recognised code language.
///
/// Used by [`crate::detect`] to decide whether a single file should
/// default to code mode, or whether a directory contains enough
/// real source to warrant code-mode routing.
pub(crate) fn is_recognized_code(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| CODE_EXTENSIONS.iter().any(|e| *e == ext))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn known_extensions_round_trip() {
        for (ext, expected_lang) in EXT_TO_LANG {
            let p = PathBuf::from(format!("foo.{ext}"));
            assert_eq!(
                lang_for(&p),
                *expected_lang,
                "extension '{ext}' should map to {expected_lang}",
            );
        }
    }

    #[test]
    fn unknown_extension_falls_back_to_text() {
        assert_eq!(lang_for(&PathBuf::from("foo.unknown")), "text");
    }

    #[test]
    fn no_extension_falls_back_to_text() {
        assert_eq!(lang_for(&PathBuf::from("README")), "text");
    }

    #[test]
    fn extension_is_case_insensitive() {
        assert_eq!(lang_for(&PathBuf::from("App.TSX")), "typescript");
    }

    #[test]
    fn recognized_code_distinguishes_known_vs_unknown() {
        assert!(is_recognized_code(&PathBuf::from("main.rs")));
        assert!(is_recognized_code(&PathBuf::from("app.vue")));
        assert!(!is_recognized_code(&PathBuf::from("README")));
        assert!(!is_recognized_code(&PathBuf::from("photo.png")));
    }

    #[test]
    fn recognized_code_excludes_plain_text_and_config() {
        // Text / config formats have a CodeMirror lang ("text"/"toml") so
        // they render fine when bundled, but they should not on their own
        // trigger code-mode routing.
        assert!(!is_recognized_code(&PathBuf::from("note.txt")));
        assert!(!is_recognized_code(&PathBuf::from("debug.log")));
        assert!(!is_recognized_code(&PathBuf::from("Cargo.toml")));
        assert!(!is_recognized_code(&PathBuf::from(".env")));
        assert!(!is_recognized_code(&PathBuf::from("data.json")));
    }
}
