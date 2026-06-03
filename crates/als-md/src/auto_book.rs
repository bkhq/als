//! Auto-bootstrap an mdbook project from raw Markdown input.
//!
//! Given either a directory of scattered `*.md` files (no `book.toml`)
//! or a single `*.md` file, lay out a scratch directory that
//! `mdbook::MDBook::load` can consume:
//!
//! ```text
//! <scratch>/
//! ├── book.toml
//! └── src/
//!     ├── SUMMARY.md          (generated)
//!     ├── <input files copied verbatim, .md and assets alike>
//! ```
//!
//! `SUMMARY.md` lists every Markdown file:
//! - `index.md` first if present (root of the book),
//! - then the rest in path-alphabetical order,
//! - chapter titles taken from the first ATX-style `# H1` of each file,
//!   falling back to the filename stem when no H1 is present.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use als_core::Error;

/// Filename written under the book root and referenced from
/// `book.toml`'s `additional-css` so mdbook layers it on top of the
/// built-in theme.
const ALS_THEME_CSS_FILE: &str = "toss-theme.css";

/// CSS body embedded at compile time. See `src/toss_theme.css` for the
/// canonical source. Stays small (~3 KB) and intentionally only tweaks
/// typography + tables + code blocks; the rest of mdbook's default
/// theme (sidebar, dark mode, search, print) is untouched.
const ALS_THEME_CSS: &str = include_str!("toss_theme.css");

/// Single-page overlay filename + body. Only emitted on the
/// `bootstrap_from_file` path so a one-chapter book hides its
/// otherwise-useless sidebar and prev/next strip.
const ALS_SINGLE_CSS_FILE: &str = "toss-single.css";
const ALS_SINGLE_CSS: &str = include_str!("toss_single.css");

/// Per-page TOC builder. Emitted on every auto-bootstrap path and
/// referenced from `book.toml`'s `additional-js`. The script scans
/// each page's `h2` / `h3` headings at load time and inserts a TOC
/// panel under the first `h1` when at least two headings are present.
const ALS_THEME_JS_FILE: &str = "toss-theme.js";
const ALS_THEME_JS: &str = include_str!("toss_theme.js");

/// Lay out `scratch` as a complete mdbook project rooted in the directory
/// of Markdown files at `root`. Returns the path to `scratch` so the
/// caller can hand it to `mdbook::MDBook::load`.
pub(crate) fn bootstrap_from_dir(root: &Path, scratch: &Path) -> Result<(), Error> {
    let src = scratch.join("src");
    fs::create_dir_all(&src).map_err(io_err("create scratch src"))?;

    let matcher = als_pack::ignore::build_matcher(root)?;
    let entries: Vec<als_pack::Entry> =
        als_pack::walk::walk(root, &matcher).collect::<Result<_, _>>()?;

    let mut chapters: Vec<Chapter> = Vec::new();
    for entry in entries {
        let dest = src.join(&entry.rel_path);
        ensure_parent(&dest)?;
        fs::copy(&entry.abs_path, &dest).map_err(io_err("copy source file"))?;

        if is_markdown(&entry.rel_path) {
            let source = fs::read_to_string(&entry.abs_path).map_err(io_err("read markdown"))?;
            let title = extract_h1(&source).unwrap_or_else(|| stem(&entry.rel_path));
            chapters.push(Chapter {
                rel_path: entry.rel_path.clone(),
                title,
            });
        }
    }

    write_summary(&src, &chapters)?;
    write_theme_css(scratch)?;
    write_theme_js(scratch)?;
    write_book_toml(scratch, &basename_title(root), &[])?;
    Ok(())
}

/// Lay out `scratch` as a single-page mdbook project around `file`.
/// The file is copied to `src/index.md` so the produced site has its
/// content at the root URL.
pub(crate) fn bootstrap_from_file(file: &Path, scratch: &Path) -> Result<(), Error> {
    let src = scratch.join("src");
    fs::create_dir_all(&src).map_err(io_err("create scratch src"))?;
    fs::copy(file, src.join("index.md")).map_err(io_err("copy single markdown"))?;

    let source = fs::read_to_string(file).map_err(io_err("read markdown"))?;
    let stem_label = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("page")
        .to_owned();
    let title = extract_h1(&source).unwrap_or_else(|| stem_label.clone());

    let chapters = vec![Chapter {
        rel_path: PathBuf::from("index.md"),
        title: title.clone(),
    }];
    write_summary(&src, &chapters)?;
    write_theme_css(scratch)?;
    write_single_css(scratch)?;
    write_theme_js(scratch)?;
    write_book_toml(scratch, &title, &[ALS_SINGLE_CSS_FILE])?;
    Ok(())
}

/// Drop the bundled `toss-theme.css` next to `book.toml`. Referenced
/// from `[output.html] additional-css = ["toss-theme.css"]`.
fn write_theme_css(scratch: &Path) -> Result<(), Error> {
    fs::write(scratch.join(ALS_THEME_CSS_FILE), ALS_THEME_CSS)
        .map_err(io_err("write toss-theme.css"))?;
    Ok(())
}

/// Drop the single-chapter overlay next to `book.toml`. Called from
/// `bootstrap_from_file` only; the dir bootstrap uses the regular
/// multi-chapter chrome.
fn write_single_css(scratch: &Path) -> Result<(), Error> {
    fs::write(scratch.join(ALS_SINGLE_CSS_FILE), ALS_SINGLE_CSS)
        .map_err(io_err("write toss-single.css"))?;
    Ok(())
}

/// Drop the per-page TOC script next to `book.toml`. Called from both
/// auto-bootstrap paths.
fn write_theme_js(scratch: &Path) -> Result<(), Error> {
    fs::write(scratch.join(ALS_THEME_JS_FILE), ALS_THEME_JS)
        .map_err(io_err("write toss-theme.js"))?;
    Ok(())
}

struct Chapter {
    rel_path: PathBuf,
    title: String,
}

/// Group key: the first path component of a chapter's relative path
/// when there is a sub-directory, else `None` for files at the root.
fn group_key(rel: &Path) -> Option<String> {
    let mut comps = rel.components();
    let first = comps.next()?;
    if comps.next().is_some() {
        Some(first.as_os_str().to_string_lossy().into_owned())
    } else {
        None
    }
}

/// Order chapters within a group: any `index.md` (at this level) first,
/// then the rest by full relative path so the listing is deterministic.
fn chapter_order(a: &Chapter, b: &Chapter) -> std::cmp::Ordering {
    fn is_index(rel: &Path) -> bool {
        rel.file_name().and_then(|n| n.to_str()) == Some("index.md")
    }
    match (is_index(&a.rel_path), is_index(&b.rel_path)) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.rel_path.cmp(&b.rel_path),
    }
}

/// Render `SUMMARY.md`.
///
/// Layout — root files take the top tier; each first-level subdirectory
/// becomes a separate mdbook "part" with its own `# <dirname>` header
/// and its files listed beneath:
///
/// ```text
/// # Summary
///
/// - [Home](./index.md)
/// - [Architecture](./architecture.md)
/// ...
///
/// # guides
///
/// - [Getting started](./guides/getting-started.md)
/// - [Configuration](./guides/configuration.md)
/// ...
/// ```
///
/// Without this grouping the sidebar mixes root-level documents in with
/// every subdirectory file alphabetically, which is rarely what a reader
/// actually wants.
fn write_summary(src: &Path, chapters: &[Chapter]) -> Result<(), Error> {
    let mut groups: BTreeMap<Option<String>, Vec<&Chapter>> = BTreeMap::new();
    for chapter in chapters {
        groups
            .entry(group_key(&chapter.rel_path))
            .or_default()
            .push(chapter);
    }
    for entries in groups.values_mut() {
        entries.sort_by(|a, b| chapter_order(a, b));
    }

    let mut body = String::from("# Summary\n\n");
    if let Some(root) = groups.remove(&None) {
        emit_entries(&mut body, &root);
    }
    for (key, entries) in &groups {
        let Some(dir) = key else { continue };
        body.push('\n');
        let _ = writeln!(body, "# {}", escape_part_title(dir));
        body.push('\n');
        emit_entries(&mut body, entries);
    }
    fs::write(src.join("SUMMARY.md"), body).map_err(io_err("write SUMMARY.md"))?;
    Ok(())
}

fn emit_entries(body: &mut String, entries: &[&Chapter]) {
    for chapter in entries {
        let href = chapter.rel_path.to_string_lossy().replace('\\', "/");
        let title = escape_md_link_text(&chapter.title);
        let _ = writeln!(body, "- [{title}](./{href})");
    }
}

/// Render a subdirectory name as an mdbook "part" title. Brackets,
/// backticks, and other Markdown syntax stay as plain text; mdbook
/// reads the line literally between `#` and the newline.
fn escape_part_title(name: &str) -> String {
    name.replace('\\', "/").replace('\n', " ")
}

fn write_book_toml(scratch: &Path, title: &str, extra_css: &[&str]) -> Result<(), Error> {
    let mut css_files: Vec<&str> = vec![ALS_THEME_CSS_FILE];
    css_files.extend_from_slice(extra_css);
    let css_list = css_files
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!(
        "[book]\n\
         title = \"{title}\"\n\
         src = \"src\"\n\
         \n\
         [output.html]\n\
         default-theme = \"light\"\n\
         preferred-dark-theme = \"navy\"\n\
         additional-css = [{css_list}]\n\
         additional-js = [\"{js}\"]\n",
        title = escape_toml_string(title),
        css_list = css_list,
        js = ALS_THEME_JS_FILE,
    );
    fs::write(scratch.join("book.toml"), toml).map_err(io_err("write book.toml"))?;
    Ok(())
}

/// First `# H1` line of an ATX-style heading. Setext headings (`Title`
/// underlined with `===`) are intentionally not handled — uncommon, and
/// fallback to filename stem keeps the auto-bootstrap deterministic.
fn extract_h1(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim_start();
        // Single `#` followed by space; reject `## H2` and `#No-space`.
        let Some(rest) = trimmed.strip_prefix('#') else {
            continue;
        };
        if rest.starts_with('#') {
            continue;
        }
        let title = rest.trim();
        if !title.is_empty() {
            return Some(title.to_owned());
        }
    }
    None
}

fn basename_title(root: &Path) -> String {
    let canonical = fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root));
    canonical
        .file_name()
        .map_or_else(|| "Site".into(), |n| n.to_string_lossy().into_owned())
}

fn stem(rel: &Path) -> String {
    rel.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_owned()
}

fn is_markdown(rel: &Path) -> bool {
    rel.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

fn ensure_parent(dest: &Path) -> Result<(), Error> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(io_err("create parent"))?;
    }
    Ok(())
}

fn escape_md_link_text(s: &str) -> String {
    // Inside `[...](...)` link text, `]` and `[` would unbalance the
    // brackets. Replace with their HTML entities; mdbook renders them
    // back to characters in the final output.
    s.replace('[', "&#91;").replace(']', "&#93;")
}

fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn io_err(ctx: &'static str) -> impl Fn(std::io::Error) -> Error {
    move |e| Error::Other(format!("{ctx}: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn extract_h1_atx_basic() {
        assert_eq!(extract_h1("# Title\n\nbody").as_deref(), Some("Title"));
    }

    #[test]
    fn extract_h1_skips_h2_and_picks_first_h1() {
        let src = "## Sub\nmore\n# Real Title\n";
        assert_eq!(extract_h1(src).as_deref(), Some("Real Title"));
    }

    #[test]
    fn extract_h1_returns_none_when_absent() {
        assert!(extract_h1("just body\n## sub only\n").is_none());
    }

    #[test]
    fn bootstrap_dir_emits_book_toml_and_summary() {
        let src = tempdir().unwrap();
        fs::write(src.path().join("index.md"), "# Home\n\nhi\n").unwrap();
        fs::write(src.path().join("other.md"), "# Other\n").unwrap();
        fs::write(src.path().join("logo.svg"), b"<svg/>").unwrap();

        let scratch = tempdir().unwrap();
        bootstrap_from_dir(src.path(), scratch.path()).unwrap();

        let book_toml = fs::read_to_string(scratch.path().join("book.toml")).unwrap();
        assert!(book_toml.contains("[book]"));
        assert!(book_toml.contains("src = \"src\""));
        // Theme is layered via additional-css; the CSS file lives next to book.toml.
        // Dir bootstrap uses only the theme overlay; the single-page overlay must
        // not be referenced from the multi-chapter path.
        assert!(book_toml.contains("additional-css = [\"toss-theme.css\"]"));
        assert!(book_toml.contains("additional-js = [\"toss-theme.js\"]"));
        assert!(!book_toml.contains("toss-single.css"));
        assert!(!scratch.path().join("toss-single.css").exists());
        let theme = fs::read_to_string(scratch.path().join("toss-theme.css")).unwrap();
        assert!(
            theme.contains(".content"),
            "embedded theme should target mdbook's .content element",
        );
        let toc_js = fs::read_to_string(scratch.path().join("toss-theme.js")).unwrap();
        assert!(
            toc_js.contains("toss-toc"),
            "embedded JS should build the .toss-toc panel",
        );

        let summary = fs::read_to_string(scratch.path().join("src/SUMMARY.md")).unwrap();
        // index.md must come first, before other.md alphabetically.
        let i_idx = summary.find("index.md").expect("index in summary");
        let o_idx = summary.find("other.md").expect("other in summary");
        assert!(i_idx < o_idx, "index.md should precede other.md");
        assert!(summary.contains("[Home]"));
        assert!(summary.contains("[Other]"));

        // Assets copied verbatim into src/.
        let asset = fs::read(scratch.path().join("src/logo.svg")).unwrap();
        assert_eq!(asset, b"<svg/>");
    }

    #[test]
    fn summary_groups_root_files_before_subdir_parts() {
        let src = tempdir().unwrap();
        // Root-level Markdown
        fs::write(src.path().join("index.md"), "# Home\n").unwrap();
        fs::write(src.path().join("architecture.md"), "# Architecture\n").unwrap();
        fs::write(src.path().join("overview.md"), "# Overview\n").unwrap();
        // Sub-dir Markdown — two distinct dirs.
        fs::create_dir(src.path().join("guides")).unwrap();
        fs::write(
            src.path().join("guides/getting-started.md"),
            "# Getting started\n",
        )
        .unwrap();
        fs::write(
            src.path().join("guides/configuration.md"),
            "# Configuration\n",
        )
        .unwrap();
        fs::create_dir(src.path().join("reference")).unwrap();
        fs::write(src.path().join("reference/cli.md"), "# CLI\n").unwrap();

        let scratch = tempdir().unwrap();
        bootstrap_from_dir(src.path(), scratch.path()).unwrap();
        let summary = fs::read_to_string(scratch.path().join("src/SUMMARY.md")).unwrap();

        // Root-level chapters appear before any `# <dirname>` part header.
        let index_pos = summary.find("(./index.md)").expect("index.md");
        let arch_pos = summary
            .find("(./architecture.md)")
            .expect("architecture.md");
        let overview_pos = summary.find("(./overview.md)").expect("overview.md");
        let guides_header = summary.find("# guides").expect("guides part header");
        let reference_header = summary.find("# reference").expect("reference part header");
        let guides_entry = summary
            .find("(./guides/getting-started.md)")
            .expect("guides entry");
        let reference_entry = summary
            .find("(./reference/cli.md)")
            .expect("reference entry");

        // index.md is the first chapter; both index.md and other root files
        // sit above the first subdirectory part header.
        assert!(index_pos < arch_pos, "index.md must come first");
        assert!(arch_pos < guides_header);
        assert!(overview_pos < guides_header);
        // Parts ordered alphabetically by dir name; their entries follow.
        assert!(guides_header < reference_header);
        assert!(guides_header < guides_entry);
        assert!(reference_header < reference_entry);
        assert!(
            guides_entry < reference_header,
            "guides entries stay under guides part"
        );
    }

    #[test]
    fn bootstrap_file_emits_single_page_overlay() {
        let src = tempdir().unwrap();
        let f = src.path().join("notes.md");
        fs::write(&f, "# Notes\n\nbody\n").unwrap();

        let scratch = tempdir().unwrap();
        bootstrap_from_file(&f, scratch.path()).unwrap();

        let book_toml = fs::read_to_string(scratch.path().join("book.toml")).unwrap();
        // additional-css must list both layers, in order (theme first, single-page second).
        assert!(
            book_toml.contains("additional-css = [\"toss-theme.css\", \"toss-single.css\"]"),
            "book.toml additional-css missing the single-page overlay: {book_toml}",
        );
        // additional-js is identical for both auto-bootstrap paths.
        assert!(book_toml.contains("additional-js = [\"toss-theme.js\"]"));
        // Both CSS files must exist on disk next to book.toml.
        let single = fs::read_to_string(scratch.path().join("toss-single.css")).unwrap();
        assert!(
            single.contains("#sidebar"),
            "single-page overlay should hide the sidebar",
        );
        assert!(scratch.path().join("toss-theme.js").is_file());
    }

    #[test]
    fn bootstrap_file_writes_index_md_and_summary() {
        let src = tempdir().unwrap();
        let f = src.path().join("notes.md");
        fs::write(&f, "# Notes\n\nBody.\n").unwrap();

        let scratch = tempdir().unwrap();
        bootstrap_from_file(&f, scratch.path()).unwrap();

        let copied = fs::read_to_string(scratch.path().join("src/index.md")).unwrap();
        assert!(copied.contains("# Notes"));
        let summary = fs::read_to_string(scratch.path().join("src/SUMMARY.md")).unwrap();
        assert!(summary.contains("[Notes](./index.md)"));
        let book_toml = fs::read_to_string(scratch.path().join("book.toml")).unwrap();
        assert!(book_toml.contains("title = \"Notes\""));
    }

    #[test]
    fn escape_toml_string_handles_quotes_and_backslashes() {
        assert_eq!(escape_toml_string("a\"b\\c"), "a\\\"b\\\\c");
    }
}
