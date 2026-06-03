//! HTML shell rendering for code-mode bundles.
//!
//! One template, three substitutions: title, file list (noscript
//! fallback), and the pinned viewer CDN URL. Lots of fixed bytes
//! around them, so a simple `String::replace` chain is enough — no
//! template engine dep.

use crate::ViewerSource;
use crate::manifest::FileEntry;

const TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>{{title}}</title>
  <link rel="icon" href="data:,">
  <link rel="stylesheet" href="{{viewer_css}}">
  <style>html,body,#toss-root{margin:0;height:100%}</style>
</head>
<body>
  <div id="toss-root" data-manifest="manifest.json" data-title="{{title}}"></div>
  <noscript>
    <article style="max-width:720px;margin:2rem auto;font:14px/1.5 system-ui,sans-serif">
      <h1>{{title}}</h1>
      <p>This code site renders with JavaScript. Without it, the files are still browsable:</p>
      <ul>{{file_list}}</ul>
    </article>
  </noscript>
  <script type="module" src="{{viewer_js}}"></script>
</body>
</html>
"#;

/// Render the HTML shell for the given [`ViewerSource`]. `title` is
/// HTML-escaped; file paths in the noscript fallback are URL-escaped
/// for the `href` and HTML-escaped for the link text.
///
/// The viewer JS/CSS URLs are resolved internally from the
/// [`ViewerSource`] enum rather than taken as arbitrary `&str`
/// parameters, so the template can never be steered at an
/// attacker-controlled URL even if a future caller flowed user input
/// into a "custom viewer base" option.
pub(crate) fn render(title: &str, files: &[FileEntry], viewer: ViewerSource) -> String {
    let (viewer_js, viewer_css) = viewer_urls(viewer);
    let file_list = render_noscript_list(files);
    TEMPLATE
        .replace("{{title}}", &html_escape(title))
        .replace("{{file_list}}", &file_list)
        .replace("{{viewer_js}}", &viewer_js)
        .replace("{{viewer_css}}", &viewer_css)
}

/// Resolve the viewer JS / CSS URLs (or relative paths) the HTML shell
/// should reference for `viewer`. Output strings are always under crate
/// control: a CDN URL composed from the pinned constants, or fixed
/// relative paths to the embedded copies.
fn viewer_urls(viewer: ViewerSource) -> (String, String) {
    match viewer {
        ViewerSource::Cdn => (crate::viewer_cdn_url(), crate::viewer_css_cdn_url()),
        ViewerSource::LocalEmbedded => ("./viewer.js".to_owned(), "./viewer.css".to_owned()),
    }
}

fn render_noscript_list(files: &[FileEntry]) -> String {
    let mut out = String::new();
    for f in files {
        out.push_str("<li><a href=\"");
        out.push_str("raw/");
        out.push_str(&url_escape_path(&f.path));
        out.push_str("\">");
        out.push_str(&html_escape(&f.path));
        out.push_str("</a></li>");
    }
    out
}

/// Minimal HTML attribute / text escape. Sufficient for titles and
/// path text inside `<a>` and `<li>`.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Percent-encode every path segment but keep `/` separators.
fn url_escape_path(path: &str) -> String {
    path.split('/')
        .map(url_escape_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn url_escape_segment(seg: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(seg.len());
    for byte in seg.bytes() {
        if is_url_unreserved(byte) {
            out.push(byte as char);
        } else {
            // Writing into a String never errors; surface any panic to
            // catch buggy contracts during dev rather than silently lose.
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

const fn is_url_unreserved(b: u8) -> bool {
    matches!(b,
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~')
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::manifest::FileEntry;

    fn entry(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            size: 0,
            lang: "text".into(),
        }
    }

    fn cdn_render(title: &str, files: &[FileEntry]) -> String {
        render(title, files, ViewerSource::Cdn)
    }

    #[test]
    fn html_escape_handles_special_chars() {
        assert_eq!(
            html_escape("Q3 & <Acme> \"v1\""),
            "Q3 &amp; &lt;Acme&gt; &quot;v1&quot;"
        );
    }

    #[test]
    fn url_escape_keeps_slash_and_safe_chars() {
        assert_eq!(url_escape_path("src/main.rs"), "src/main.rs");
        assert_eq!(url_escape_path("a b/c d.txt"), "a%20b/c%20d.txt");
        assert_eq!(url_escape_path("weird#path?.ts"), "weird%23path%3F.ts");
    }

    #[test]
    fn render_interpolates_title_and_cdn() {
        let html = cdn_render("My Demo", &[entry("a.ts")]);
        assert!(html.contains("<title>My Demo</title>"));
        assert!(html.contains(&crate::viewer_cdn_url()));
        assert!(html.contains("toss-code-viewer@"));
        assert!(html.contains("viewer.css"));
    }

    #[test]
    fn render_with_local_paths_references_relative_urls() {
        let html = render("t", &[entry("a.ts")], ViewerSource::LocalEmbedded);
        assert!(html.contains("src=\"./viewer.js\""));
        assert!(html.contains("href=\"./viewer.css\""));
        assert!(
            !html.contains("cdn.jsdelivr.net"),
            "local mode must not reference any CDN: {html}"
        );
    }

    #[test]
    fn render_escapes_title_in_html() {
        let html = cdn_render("<script>x</script>", &[]);
        assert!(!html.contains("<script>x</script>"));
        assert!(html.contains("&lt;script&gt;x&lt;/script&gt;"));
    }

    #[test]
    fn render_noscript_lists_files() {
        let html = cdn_render("t", &[entry("src/main.rs"), entry("Cargo.toml")]);
        assert!(html.contains("href=\"raw/src/main.rs\""));
        assert!(html.contains("href=\"raw/Cargo.toml\""));
    }

    #[test]
    fn render_noscript_url_escapes_paths() {
        let html = cdn_render("t", &[entry("a b/c.rs")]);
        assert!(html.contains("href=\"raw/a%20b/c.rs\""));
        // Original path text is still html-escaped (not url-escaped) in the
        // visible <a> contents.
        assert!(html.contains(">a b/c.rs<"));
    }
}
