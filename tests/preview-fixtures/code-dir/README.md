# als preview code-dir fixture

This file lives inside a **code-mode bundle** (forced via `--kind code`),
not a markdown site. The viewer opens it in *rendered* mode by default
and exposes a "Source" toggle so reviewers can flip between:

- **Rendered (preview)** — `marked` → sanitised HTML in a styled article.
- **Source (edit)** — the raw text in the CodeMirror editor pane.

## Files in this fixture

1. `README.md` — this file. Demonstrates the Markdown rendered/source toggle.
2. `page.html` — demonstrates the HTML iframe rendered/source toggle.
3. `src/main.rs` — plain Rust source; only has the CodeMirror pane.
4. `src/util.py` — plain Python source; only has the CodeMirror pane.

```rust
// Example code block to verify syntax-highlighted fences render.
fn main() {
    println!("hello");
}
```
