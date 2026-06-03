# als-md

Markdown + mdbook pre-render layer for the als CLI. Consumed by both
the deploy path (`als <path>`) and the local preview path
(`als preview`).

## Two-stage flow

```rust
use als_md::{classify, render, MdInput};

let Some(input) = als_md::classify(path)? else {
    // Not markdown / mdbook — fall through to als-pack.
    return Ok(None);
};
let rendered = als_md::render(input)?;
// rendered.path() is a tempdir of plain HTML.
// rendered.stem is the suggested archive basename.
```

### `classify(path) -> Option<MdInput>`

Returns `Some(...)` for:

| Input                                        | Variant                             |
|----------------------------------------------|-------------------------------------|
| Directory containing `book.toml`             | `MdInput::MdBook(path)`             |
| Directory with at least one `*.md` (after `als.toml` exclude filtering) | `MdInput::AutoMdBook(Directory)` |
| Single `*.md` file                           | `MdInput::AutoMdBook(File)`         |

Anything else returns `None` so the binary falls through to the
existing folder / zip / single-HTML paths.

### `render(input) -> Rendered`

- `MdBook` — runs `mdbook::MDBook::build` against the author's tree
  unmodified; their `book.toml` and `SUMMARY.md` are honoured.
- `AutoMdBook(Directory)` — synthesises a `book.toml` and a
  `SUMMARY.md` in a scratch tempdir (root-level files listed
  alphabetically first, then one `# <dirname>` part per
  sub-directory), then builds.
- `AutoMdBook(File)` — synthesises a one-chapter book with that
  file as the only entry, applies the `toss_single.css` overlay
  (sidebar collapse), then builds.

The scratch tempdir is dropped immediately after the build; only
the final output tempdir (held by `Rendered`) lives on.

## Bundled theme

The auto-bootstrap path always layers the bundled assets on top of
mdbook's default theme:

| File             | Purpose                                                                |
|------------------|------------------------------------------------------------------------|
| `toss_theme.css` | Typography polish, table / code-block / blockquote styling, right-rail per-page TOC, pill prev / next nav at the bottom of every page. |
| `toss_theme.js`  | Builds the per-page TOC by scanning `h2` / `h3` in `.content`. Skipped if the page already includes a `nav.toss-toc` (author opt-out). |
| `toss_single.css`| Only for `AutoMdBook(File)`: hides the empty sidebar and the prev / next strip since neither is meaningful for a one-chapter book. |

Author-managed `book.toml` projects are rendered verbatim — no
overlay, no SUMMARY synthesis.

## Lifecycle ownership

`Rendered` owns a `TempDir` (`tempfile`). The caller (`als preview`
or `als <path>` deploy pipeline) holds it for the duration of the
operation; `Drop` recursively removes the tree. `commands/preview.rs`
wraps it in a `PreviewGuard::Rendered(...)` enum that also covers the
zip-extracted and single-html cases.

## Tests

Unit tests under `#[cfg(test)]` in each module cover:

- `classify`: every input shape (book.toml-only dir, markdown-only
  dir, single file, no-match dir).
- `auto_book::bootstrap_from_dir` / `bootstrap_from_file`: synth
  SUMMARY layout (root files first, sub-dirs as parts).
- `auto_book::group_key` / `chapter_order` / `emit_entries`: the
  sort and grouping helpers exercised in isolation.

Run with `cargo test -p als-md --lib`.
