# Architecture

Snapshot of the als CLI workspace structure, module boundaries, and
testing architecture. Not a tutorial — see [`cli-spec.md`](cli-spec.md)
for product behaviour and the `cli-server-contract.md` in the
als-docs repo for the server contract. For local-build workflow see
[`development.md`](development.md); for the typed client surface see
[`api-reference.md`](api-reference.md).

## Workspace

Seven crates, single-direction dependency graph:

```
als (bin)
  ├─ als-core         error / config / output / duration
  ├─ als-api     ───▶ als-core
  ├─ als-pack    ───▶ als-core
  ├─ als-md      ───▶ als-core    (mdbook pre-render)
  ├─ als-preview ───▶ als-core    (tiny_http preview server)
  └─ als-code ──▶ als-core, als-pack
                                    (CodeMirror viewer bundle)
```

| Crate            | Kind | Responsibility |
|------------------|------|----------------|
| `als-core`      | lib  | TOML config + env override + profile, `thiserror` error enum + `ExitCode` mapping, `OutputMode` (Human / Quiet / Json), `Duration` parser. No reqwest, no zip, no clap. |
| `als-api`       | lib  | `reqwest` client builder with Bearer injection, `{ success, data \| error }` envelope, typed endpoint wrappers under `endpoints/`. No zip, no clap, no TTY logic. |
| `als-pack`      | lib  | Default ignore set + `als.toml.exclude` merge, `walkdir`-based collection, zip writing (deflate-flate2 only). Hosts the canonical `AlsConfig` schema. No reqwest, no clap. |
| `als-md`        | lib  | Classify markdown / mdbook inputs and pre-render them through `mdbook` into a tempdir of HTML. Auto-bootstraps `book.toml` + `SUMMARY.md` for plain markdown directories or single `.md` files. No reqwest, no clap. |
| `als-preview`   | lib  | Pure-Rust static-file HTTP server on `tiny_http`. GET / HEAD only, MIME table, percent-decoded path resolve, `ShutdownHandle` for graceful stop. No mdbook, no zip — orchestration of input shapes lives in the binary. |
| `als-code`  | lib  | Detect pure-code inputs (single recognised code file, or directory of code with no `.html` / `.md` / `book.toml`), apply 4 limits (50 files / 256 KB / 2 MB / depth 8), and produce a tempdir bundle (`index.html` + `manifest.json` + `raw/*`). The generated HTML references the `toss-code-viewer` npm package on jsdelivr by pinned version — the published site is browser-rendered with CodeMirror 6 at view time. No reqwest, no clap. |
| `als`           | bin  | `tokio::main`, clap parsing, command handlers under `commands/`. Composes the six lib crates. The only place printing is allowed (via `als-core::output`). |

Dependency rationale:

- `als-api` reuses `als-core::Error` so HTTP errors map naturally to
  exit codes.
- `als-pack`, `als-md`, `als-preview` reuse `als-core::Error` for
  IO failures.
- Lib crates do not depend on each other beyond `als-core` — keeps
  unit tests fast and avoids accidental coupling between API,
  packaging, rendering, and preview concerns.

## Module Layout

```
crates/
├── als-core/src/
│   ├── lib.rs
│   ├── config.rs        TOML + env + profile (etcetera for XDG)
│   ├── error.rs         thiserror enum + ExitCode trait
│   ├── output.rs        OutputMode + anstream wiring + JSON helpers
│   ├── duration.rs      "5m" / "24h" / "7d" / "never" / RFC 3339
│   └── sites.rs         per-site pin store under `sites/<id>_<name>.toml`
├── als-api/src/
│   ├── lib.rs
│   ├── client.rs        reqwest builder + Bearer + envelope unwrap
│   ├── envelope.rs      { success, data | error } typed deserialization
│   ├── error.rs         ApiError + From<ApiError> for als-core::Error
│   └── endpoints/
│       ├── mod.rs
│       ├── account.rs   GET account/me
│       ├── quota.rs     GET quota/me
│       ├── device.rs    POST auth/device/{authorize,token} (RFC 8628)
│       ├── deploy.rs    POST deploy (multipart)
│       ├── sites.rs     GET / GET:id / PATCH:id / DELETE:id
│       ├── password.rs  POST sites/:id/password
│       ├── versions.rs  GET sites/:id/versions, POST sites/:id/activate
│       └── tokens.rs    DELETE tokens/:prefix
├── als-pack/src/
│   ├── lib.rs           classify / pack entry points + 50 MB ceiling
│   ├── config.rs        canonical `als.toml` schema (`AlsConfig`)
│   ├── digest.rs        SHA-256 source fingerprint for the no-op skip
│   ├── ignore.rs        Default ignore set + `als.toml.exclude` merge
│   ├── walk.rs          walkdir + ignore filter
│   └── zip.rs           Zip writer (deflate-flate2)
├── als-md/src/
│   ├── lib.rs           classify + render entry points
│   ├── auto_book.rs     Synth book.toml + SUMMARY.md for bare markdown
│   ├── toss_theme.css   Bundled mdbook polish (always applied)
│   ├── toss_theme.js    Per-page TOC (right-rail, fixed)
│   └── toss_single.css  Single-file overlay (hides empty sidebar)
├── als-preview/src/
│   ├── lib.rs           PreviewServer + ShutdownHandle, request loop
│   ├── mime.rs          Static MIME lookup table
│   └── path_resolve.rs  Percent-decoded path resolver (zip-slip safe)
├── als-code/
│   ├── assets/
│   │   ├── viewer.js    snapshot of `../../../../viewer/viewer-lite.js`,
│   │   │                refreshed via `just sync-viewer` and embedded
│   │   │                into the binary via `include_str!`.
│   │   └── viewer.css   sibling snapshot.
│   └── src/
│       ├── lib.rs       public API + VIEWER_PKG / VIEWER_VERSION constants
│       │                (re-exports `AlsConfig` from `als-pack`)
│       ├── detect.rs    classify (auto) + force_input (--kind override)
│       ├── walker.rs    bounded walk (limits + binary skip)
│       ├── lang.rs      extension → CodeMirror language id table
│       ├── manifest.rs  manifest.json serde shape
│       ├── template.rs  HTML shell + html / url escape helpers
│       └── bundle.rs    tempdir assembly (index.html + manifest.json + raw/)
└── als/src/
    ├── main.rs          tokio runtime + command dispatch
    ├── cli.rs           clap derive structures + KindFlag
    ├── output.rs        the one place workspace-deny printing is allowed
    ├── prompt.rs        shared `confirm` helper (TTY + non-TTY fallback)
    └── commands/
        ├── mod.rs
        ├── auth.rs      login / logout / revoke / status
        ├── completion.rs, config.rs
        ├── deploy.rs    bare `als <path>` upload pipeline + resolve_kind router
        ├── list.rs, rm.rs, site.rs, site_ident.rs, unpin.rs
        ├── preview.rs   orchestrates als-md / als-code / als-pack inputs
        └── qr.rs        terminal QR for login one-tap URL
```

Each file targets 200-400 lines (hard cap 800).

## Preview Pipeline (als-md + als-preview + als-code)

`commands/preview.rs::resolve_preview_root` is the dispatcher:

| Input                                       | Path                            | Tempdir? |
|---------------------------------------------|---------------------------------|----------|
| Folder with `index.html`, no `.md`          | `PreviewGuard::Plain`           | no       |
| Folder with `book.toml`                     | `MdInput::MdBook` → render      | yes      |
| Folder with `*.md`, no `book.toml`          | `MdInput::AutoMdBook(Directory)`| yes      |
| Single `*.md` file                          | `MdInput::AutoMdBook(File)`     | yes      |
| Single `*.html` file                        | Copied as `tempdir/index.html`  | yes      |
| `*.zip` archive                             | Extracted into tempdir          | yes      |
| Pure-code folder / single code file         | `als-code` bundle with `ViewerSource::LocalEmbedded` | yes |

Tempdirs live for the lifetime of `_guard` in
`commands::preview::run`. On SIGINT / SIGTERM the runtime calls
`ShutdownHandle::shutdown()` → `tiny_http::Server::unblock` → the
request loop returns cleanly → the function unwinds → `Drop` on
`TempDir` / `als_md::Rendered` recursively removes the directories.
`SIGKILL` bypasses `Drop`; the system `tmpfiles.d` sweep is the
fallback.

The shared `als-md` `toss-theme.css` / `toss_theme.js` overlay is
injected via mdbook's `additional-css` / `additional-js` only on the
auto-bootstrap path. Author-managed `book.toml` projects are
rendered untouched.

## Testing Architecture

Three layers, bottom two share the same Bun mock server:

```
Layer 3   CLI E2E       crates/als/tests/         assert_cmd + Bun mock
Layer 2   API integ.    crates/als-api/tests/     reqwest + Bun mock
Layer 1   Unit          #[cfg(test)] in each lib   no IO
```

Plus a fourth, format-specific smoke layer for `als preview`:

```
Layer 0.5 Preview sweep tests/preview-fixtures/    `just preview-sweep`
```

`preview-sweep` boots each hand-authored fixture (HTML folder, .zip,
.html, .md, md-directory, real mdbook) on a temp port, curls `/`,
expects HTTP 200 + a known marker, kills the process. First failure
short-circuits.

### Bun Mock Server

`tests/mock-server/` runs Bun + Hono and mirrors the `cli-server-contract.md` in the als-docs repo.
Same framework as production als-api, so mock semantics track real
semantics.

Handshake: stdout first line `LISTENING http://127.0.0.1:<port>\n`.
The Rust harness reads it and sets `ALS_API`. Bun's stdin is kept
piped open by the harness; when the harness exits, `read(stdin)`
returns EOF and Bun exits, so orphans don't accumulate even when the
test binary is `SIGKILL`-ed.

Test hooks (not in production API):

- `POST /__test/reset` — clear in-memory state.
- `POST /__test/seed`  — inject sites / tokens fixtures.
- `POST /__test/inject` — one-shot failure injection (status + error code).
- `GET  /__test/state` — snapshot for debugging.
- `POST /__test/device/approve` — drive a device-flow grant
  (`approve` / `deny` / `slow_down_once` / `expire`).

State isolation: one Bun process per test binary (via `OnceCell`),
tests serialized inside the binary by `serial_test` plus
`--test-threads=1` in the workspace test runner. Server binds to
`127.0.0.1` only.

### Why Bun + TS Instead of `wiremock`

- Same stack as production server (Hono on Bun) — no semantic drift.
- Single mock implementation drives both API integration and CLI E2E
  layers.
- Real HTTP listener — exercises full reqwest / TLS / multipart
  paths.
- Complex stateful endpoints (deploy multipart, device-flow state
  machine) are clearer in TypeScript than in builder chains.

## Key Decisions

| Decision | Reason |
|----------|--------|
| 7 crates (6 lib + 1 bin) | Reusable `als-api` for future tooling; faster incremental builds; per-crate lint scoping. `als-md`, `als-preview`, and `als-code` carved off so their dep graph (mdbook, tiny_http, viewer assets) doesn't leak into core paths. |
| `thiserror` enum, no `anyhow` | Strong error code → exit code mapping per `cli-spec.md`. |
| `etcetera` for XDG | Pure-Rust, cross-platform, no `dirs` C bindings. |
| `zip` with `default-features = false` + `deflate-flate2` | Pure-Rust deflate; `bzip2-sys` / `xz2-sys` / `zstd-sys` forbidden by the pure-Rust stack rule. |
| `anstream` over manual TTY detection | Auto-strip ANSI on pipes; less surface to test. |
| Hand-rolled column-padded table in `als list` | A 30-LOC `render_table` is enough for the one consumer; saves ~40 KB of derive-based table-library code. |
| `dialoguer` for `rm` / `auth revoke` confirms | Stdin injection works in `assert_cmd`. |
| `mdbook` as the only markdown renderer | Single output style; auto-bootstrap path synthesises `book.toml` + `SUMMARY.md` so authors don't need either. |
| `tiny_http` over `hyper`/`axum` for preview | Sync API, blocking iterator, no async machinery needed; 1500-LOC dep vs hyper's stack; SIGINT-safe via `Server::unblock`. |
| `tokio::signal::ctrl_c` + Unix `SignalKind::terminate` | Native SIGINT/SIGTERM handler in the binary calls `ShutdownHandle::shutdown` so the request loop returns and tempdirs drop cleanly. |
| Two viewer bundles (`viewer/viewer.js` + `viewer/viewer-lite.js`) | The CLI binary embeds the lite bundle (5 lang packs, ~630 KB) via `include_str!`; deployed code sites load the full bundle (16 lang packs, ~1.1 MB) from the npm-published `toss-code-viewer` via jsdelivr. Keeps the binary small without regressing language coverage for end users. |
| Bun mock server | Same stack as production; single mock for two test layers. |
| Workspace lints layered (`-2 / -1 / 0` priority) | Project-wide deny on warnings, with targeted exceptions only at the call site. |

## Profiles

| Profile | Purpose |
|---------|---------|
| `dev`     | Fast iteration; deps optimized at `opt-level = 1`. |
| `release` | Production line debug, `panic = "unwind"` so backtraces resolve. |
| `dist`    | `opt-level = "z"`, `strip = "symbols"`, `panic = "abort"` — distributed binaries. |

## Distribution Targets

Per `cli-spec.md`:

- `als-linux-x64`, `als-linux-arm64` — `*-unknown-linux-musl` static binaries.
- `als-darwin-x64`, `als-darwin-arm64` — `*-apple-darwin`.
- `als-windows-x64.exe` — `*-pc-windows-msvc`.
