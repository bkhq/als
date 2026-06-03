# als preview single-md fixture

Served via the auto-bootstrap **single-file** path. The bootstrap
generates a one-chapter `book.toml`, drops the `toss-single` overlay
CSS so the empty sidebar collapses, and lets mdbook render this page
through the shared `toss-theme` polish.

## What this fixture proves

- The `.md` extension is detected by `als preview`.
- The single-file overlay (`toss-single.css`) hides the empty sidebar.
- The shared `toss-theme.css` polish still applies (typography,
  link styling, code blocks).

## TOC probe

The auto-injected per-page TOC appears when a page has at least two
`h2` / `h3` headings, which is exactly what this fixture provides.
On a wide viewport the TOC pins to the top-right of the page.

### A nested heading

So the TOC has both an `h2` and an `h3` entry.

### Another nested heading

And another, to verify the TOC builds a proper list.
