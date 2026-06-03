# als-preview fixtures

Small, hand-authored inputs covering every shape `als preview` accepts.
Pair each fixture with a `just preview-*` recipe (see the top-level
`justfile`) for an end-to-end local smoke test.

| Fixture                 | `als preview` mode                                | Recipe                |
| ----------------------- | -------------------------------------------------- | --------------------- |
| `html-dir/`             | Folder containing `index.html` (static site)      | `preview-html-dir`    |
| `single.html`           | Single `.html` file                               | `preview-html-file`   |
| `single.md`             | Single `.md` file (auto-bootstrap mdbook)         | `preview-md-file`     |
| `md-dir/`               | Markdown folder, no `book.toml` (synth SUMMARY)   | `preview-md-dir`      |
| `mdbook/`               | Real mdbook project (`book.toml` + `src/`)        | `preview-mdbook`      |
| `html-dir/` &rarr; zip  | `.zip` archive (rebuilt on demand)                | `preview-zip`         |
| `code-dir/`             | Code bundle via `--kind code`; carries `.md` + `.html` so the viewer's rendered/source toggle is exercised alongside plain source files | `preview-code-dir` |
| `single.ts`             | Single recognised code file (viewer shell)        | `preview-code-file`   |

The `.zip` fixture is materialised at
`target/preview-fixtures/html-dir.zip` from `html-dir/` so the repo
stays binary-free.

## Offline format sweep

`just preview-sweep` boots each fixture on a temp port, curls `/`,
expects HTTP 200 plus a known marker string, then kills the process.
First failure short-circuits the sweep. Use this in CI or before a
preview-related release.

## Adding a fixture

1. Add files under `tests/preview-fixtures/<name>/`.
2. Add a `preview-<name>` recipe in the justfile.
3. Add a row to `sweep.sh` so the offline check covers it.
