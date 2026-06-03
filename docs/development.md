# Development guide

Local build, test, and preview workflow for the als CLI workspace.
For the high-level structure see [`architecture.md`](architecture.md);
for the typed API client see [`api-reference.md`](api-reference.md).

## Prerequisites

| Tool      | Version  | Purpose                                                 |
|-----------|----------|---------------------------------------------------------|
| Rust      | edition 2024 (stable, see `rust-toolchain.toml`) | builds the workspace        |
| `just`    | any      | recipe runner; the canonical entry point to every task  |
| `bun`     | 1.x      | runs the mock server backing layer-2 and layer-3 tests  |
| `python3` | 3.10+    | builds the `.zip` preview fixture via `python3 -m zipfile` (only needed for `just preview-zip` / `just preview-sweep`) |
| `nsl`     | any      | optional — gives `just preview` a stable `*.localhost` / `https://*.a.wd.ds.cc/` URL |

`bun` is the only non-Rust prerequisite that is required for the test
suite; without it the layer-2 and layer-3 tests fail at process spawn.

## Build

```bash
just build       # release; produces target/release/als
just dev         # dev profile, fast incremental rebuilds
just clean       # wipe target/
```

`cargo build`, `cargo check`, etc. work directly too — `just` is a
thin wrapper.

## Quality gates (match CI)

```bash
just fmt-check   # cargo fmt --all -- --check
just lint        # cargo clippy --workspace --all-targets -- -D warnings
just check       # cargo check --workspace --all-targets
just ci          # fmt-check + lint + test (the full sweep)
```

Workspace lint policy:

- `#![forbid(unsafe_code)]` everywhere; no exceptions.
- `unwrap_used` / `expect_used` / `panic` / `print_stdout` /
  `print_stderr` are workspace-deny. Printing is allowed only inside
  the binary crate's dedicated `crates/als/src/output.rs` module.
- All repository docs, code comments, commit messages, and PR text
  are written in English.

## Testing

Three layers; see [`architecture.md#testing-architecture`](architecture.md#testing-architecture)
for the design rationale.

```bash
just test                          # everything, --test-threads=1
just test deploy_archive_too_large # filter by name
just test-md                       # als-md unit only, no bun spawn
just test-preview                  # als-preview unit only, no bun spawn
just test-e2e                      # CLI E2E binaries (needs bun)
```

`--test-threads=1` is required because the layer-2/3 tests share a
single Bun mock server and need ordered state mutations.

### Mock server lifecycle

`tests/mock-server/` (Bun + Hono) is spawned per test binary via a
shared `OnceCell` harness. The Rust side keeps Bun's stdin piped
open; when the test binary exits (cleanly or via `SIGKILL`), Bun's
`read(stdin)` returns EOF and the mock exits — so orphans don't
accumulate.

Test-only hooks:

- `POST /__test/reset`
- `POST /__test/seed`
- `POST /__test/inject` (one-shot failure injection)
- `GET  /__test/state` (snapshot for debugging)
- `POST /__test/device/approve` (drive device-flow grants)

### Device-auth integration test names

```text
login_happy_path_persists_token
login_handles_pending_then_authorized
login_handles_slow_down_then_authorized
login_reports_access_denied
login_reports_expired_token
login_with_paste_token_short_circuits
login_no_qr_omits_block_glyphs
login_prints_verification_url_for_manual_open
```

All RFC 8628 branches are covered: `authorization_pending` polling,
`slow_down` interval doubling (cap 30 s), `access_denied`,
`expired_token`, `-T` paste-token short-circuit, `--no-qr` rendering,
and the verification URL appearing on stdout for copy-paste.

Run them with `cargo test -p als --test login -- --test-threads=1`.

## Preview workflows

`als preview <path>` accepts six input shapes; see
[`architecture.md#preview-pipeline-als-md--als-preview`](architecture.md#preview-pipeline-als-md--als-preview)
for the dispatch table. The repo ships hand-authored fixtures under
`tests/preview-fixtures/` plus per-format recipes:

```bash
just preview             # default: serve current dir under nsl
just preview docs/       # serve any path under nsl
just preview-html-dir    # folder with index.html
just preview-html-file   # single .html file
just preview-md-file     # single .md file
just preview-md-dir      # markdown directory (auto-bootstrap mdbook)
just preview-mdbook      # real book.toml project
just preview-zip         # build a zip from html-dir/ on demand, serve it
just preview-code-dir    # pure-code directory (viewer shell + manifest + raw/)
just preview-code-file   # single recognised code file (viewer shell)
just preview-stop        # release the nsl route (Ctrl+C the foreground recipe too)
just preview-sweep       # offline format-coverage smoke; non-zero on first failure
```

`just preview-sweep` is suitable for CI on preview-touching PRs: it
boots every fixture on a temp port, curls `/`, expects HTTP 200 + a
known marker, and tears down. No mock server required.

The nsl wiring is opt-in. `nsl run --name als-preview ./target/release/als preview ...`
pins a stable `http://als-preview.localhost:3355/` URL plus
`https://als-preview.a.wd.ds.cc/` for cross-machine sharing. If
`nsl` isn't installed, run the binary directly:

```bash
./target/release/als preview tests/preview-fixtures/md-dir --port 8080
# then open http://127.0.0.1:8080/
```

## Adding a preview fixture

1. Add files under `tests/preview-fixtures/<name>/`.
2. Add a `preview-<name>` recipe in the `justfile`.
3. Add a row to `tests/preview-fixtures/sweep.sh` so
   `just preview-sweep` covers it.

The `.zip` fixture is materialised on demand into
`target/preview-fixtures/` (gitignored) by the private recipe
`preview-zip-build` so the repo stays binary-free.

## Adding an API endpoint

1. Add the module file under `crates/als-api/src/endpoints/<name>.rs`.
   Wrap the URL path in a typed free function that takes
   `&Client`; reuse `client.get_json` / `client.post_form` /
   `client.post_multipart` so envelope unwrap stays uniform.
2. Add typed request / response structs in the same file (camelCase
   serde, `#[serde(deny_unknown_fields)]` for response shapes).
3. Mirror the endpoint in `tests/mock-server/src/routes/<name>.ts`
   so both test layers can drive it.
4. Add a unit test (deserialization of representative payloads) in
   the new endpoint file under `#[cfg(test)]`.
5. Add a `crates/als-api/tests/<name>.rs` integration test that
   drives the real reqwest client against the bun mock.
6. Re-export the public types from `crates/als-api/src/lib.rs` if
   the binary or another crate needs them.
7. Append a row to [`api-reference.md`](api-reference.md).

## Adding a CLI subcommand

1. Add a variant to `cli::Command` and a corresponding `Args` struct
   in `crates/als/src/cli.rs`.
2. Implement the handler under
   `crates/als/src/commands/<name>.rs`. Take `&mut Output` and
   write all human / json output through it — never `print!` /
   `eprintln!`.
3. Wire the dispatch in `main.rs`.
4. Add a `#[cfg(test)]` clap-parse test alongside the args struct.
5. Add an end-to-end test in `crates/als/tests/<name>.rs` that
   spawns the binary with `assert_cmd` against the bun mock.

## Common pitfalls

- **Workspace-deny lints**: `unwrap()` and `expect()` are denied
  outside test modules. Use `?` + `Error::Other(format!(...))` for
  the bail path, or move the call into `#[cfg(test)]`.
- **Printing**: `println!` / `eprintln!` are denied everywhere except
  the binary's output module. Route through `Output::human` /
  `Output::quiet_line` / `Output::json`.
- **fmt drift**: `cargo fmt --check` is run by `just ci`. A handful
  of pre-existing import-order drifts in `crates/als/src/commands/`
  predate the current cleanup pass and were not retroactively
  reformatted — flag them in a separate PR if you want them cleaned.
- **Bun spawn failure**: layer-2/3 tests fail with `is bun on PATH?`
  if Bun isn't installed. Install via the upstream Bun installer or
  skip those layers (`just test-md` / `just test-preview`).
- **SIGKILL leaves tempdirs**: `als preview`'s tempdirs are cleaned
  on SIGINT / SIGTERM via `Drop`. `kill -9` bypasses `Drop`; the OS
  `tmpfiles.d` sweep is the eventual fallback.
