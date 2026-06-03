//! Extension -> MIME-type table. Small static array; sufficient coverage
//! for the assets mdbook + typical static-site outputs produce.

use std::path::Path;

const TABLE: &[(&str, &str)] = &[
    ("html", "text/html; charset=utf-8"),
    ("htm", "text/html; charset=utf-8"),
    ("css", "text/css; charset=utf-8"),
    ("js", "text/javascript; charset=utf-8"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("json", "application/json; charset=utf-8"),
    ("xml", "application/xml; charset=utf-8"),
    ("txt", "text/plain; charset=utf-8"),
    ("md", "text/plain; charset=utf-8"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("ico", "image/x-icon"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("ttf", "font/ttf"),
    ("otf", "font/otf"),
    ("eot", "application/vnd.ms-fontobject"),
    ("pdf", "application/pdf"),
    ("wasm", "application/wasm"),
];

const DEFAULT: &str = "application/octet-stream";

/// Returns the MIME type string for `path` based on its extension.
/// Falls back to `application/octet-stream` when the extension is
/// unknown or absent.
pub(crate) fn for_path(path: &Path) -> &'static str {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return DEFAULT;
    };
    let ext_lower = ext.to_ascii_lowercase();
    for (suffix, mime) in TABLE {
        if *suffix == ext_lower {
            return mime;
        }
    }
    DEFAULT
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn html_extension_maps_to_html_mime() {
        assert_eq!(
            for_path(&PathBuf::from("index.html")),
            "text/html; charset=utf-8"
        );
    }

    #[test]
    fn case_insensitive_extension() {
        assert_eq!(for_path(&PathBuf::from("logo.PNG")), "image/png");
    }

    #[test]
    fn unknown_extension_falls_through_to_default() {
        assert_eq!(
            for_path(&PathBuf::from("data.unknown-xyz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn extensionless_file_uses_default() {
        assert_eq!(
            for_path(&PathBuf::from("Makefile")),
            "application/octet-stream"
        );
    }
}
