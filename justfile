# `just` lists recipes. `just <recipe> [args]` runs one.
#
# Conventions:
# - All cargo commands operate on the workspace unless they target a
#   specific binary or crate.
# - `nsl` exposes long-running servers under a stable `*.localhost` URL.
#   Only `preview` needs it; one-shot CLI commands do not.

# Stable nsl app name for `just preview`. Pair with `just preview-url`
# to script-fetch the URL from CI / other tooling.
NSL_APP := "als-preview"

# Hand-authored input fixtures, one folder per preview format.
PREVIEW_FIXTURES := "tests/preview-fixtures"

# Default recipe: list everything else.
default:
    @just --list --unsorted

# ---------------------------------------------------------------------
# Build

# Optimized release build of the `als` binary. Produces target/release/als.
build:
    cargo build --release --bin als

# Fast incremental build (dev profile). For local iteration.
dev:
    cargo build --bin als

# Wipe `target/`.
clean:
    cargo clean

# ---------------------------------------------------------------------
# Quality gates (match what CI runs)

# Quick type-check across the workspace.
check:
    cargo check --workspace --all-targets

# Reformat every crate.
fmt:
    cargo fmt --all

# Verify formatting; non-zero exit on drift (pre-commit / CI).
fmt-check:
    cargo fmt --all -- --check

# Clippy across the workspace with warnings as errors.
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Full quality sweep — fmt-check + clippy + test.
ci: fmt-check lint test

# ---------------------------------------------------------------------
# Tests

# Examples:
#   just test
#   just test deploy_archive_too_large_exit_1
# Run all workspace tests (single-threaded for the shared bun mock-server harness).
test *args:
    cargo test --workspace -- --test-threads=1 {{args}}

# als-md unit tests only (fast; no bun spawn).
test-md:
    cargo test -p als-md --lib

# als-preview unit tests only (fast; no bun spawn).
test-preview:
    cargo test -p als-preview --lib

# E2E sweep over the `crates/als/tests/*.rs` binaries.
test-e2e:
    cargo test -p als --tests -- --test-threads=1

# ---------------------------------------------------------------------
# Viewer artefact sync
#
# The CodeMirror-based code viewer lives in the sibling `../viewer/`
# project (separate npm publish cycle). It ships two bundles:
#
#   - `../viewer/viewer.js`      — full (16 lang packs, ~1.1 MB).
#       Published to npm; deployed code sites load this from jsdelivr.
#   - `../viewer/viewer-lite.js` — lite (5 lang packs, ~630 KB).
#       Embedded into the `als` binary via `include_str!` so the
#       binary stays small.
#
# The CLI's `crates/als-code/assets/viewer.js` mirrors the LITE
# bundle, so a plain `cargo build` does not depend on the sibling
# project being checked out or built.
#
# Run this after editing `../viewer/src/*.js` to refresh the
# embedded snapshot:

# Rebuild both viewer bundles and refresh the (lite) als-code asset snapshot.
sync-viewer:
    cd ../viewer && bun install && bun run build:all
    cp ../viewer/viewer-lite.js crates/als-code/assets/viewer.js
    cp ../viewer/viewer.css     crates/als-code/assets/viewer.css
    @echo "Updated crates/als-code/assets/. Commit alongside any VIEWER_VERSION bump."

# Fail if the embedded (lite) asset has drifted from `../viewer/viewer-lite.js`. CI guard.
check-viewer:
    diff -q ../viewer/viewer-lite.js crates/als-code/assets/viewer.js
    diff -q ../viewer/viewer.css     crates/als-code/assets/viewer.css

# ---------------------------------------------------------------------
# Local preview (long-running)

# Examples:
#   just preview              # serve current dir
#   just preview docs/        # serve repository docs
#   just preview ./build      # serve a folder ready for deploy
# Serve <path> via als-preview through nsl on a stable URL (blocks until Ctrl+C).
preview path=".": build
    nsl run --name {{NSL_APP}} ./target/release/als preview {{path}} --bind 127.0.0.1 --port NSL_PORT

# Print the nsl URL `just preview` would register, if any.
preview-url:
    @nsl get {{NSL_APP}}

# Stop the nsl-routed preview if it is running.
preview-stop:
    nsl route delete {{NSL_APP}} 2>/dev/null || true
    @echo "If `just preview` is still running in the foreground, Ctrl+C it."

# ---------------------------------------------------------------------
# Per-format preview demos
#
# Each recipe reuses the {{NSL_APP}} nsl route so launching a new
# fixture replaces the previous one — only one preview runs at a
# time. The fixtures themselves live under {{PREVIEW_FIXTURES}} and
# cover every shape `als preview` accepts. See that directory's
# README.md for what each fixture proves.

# Folder with index.html — served as a static site.
preview-html-dir: build
    nsl run --name {{NSL_APP}} ./target/release/als preview {{PREVIEW_FIXTURES}}/html-dir --bind 127.0.0.1 --port NSL_PORT

# Single .html file — copied to a tempdir as index.html.
preview-html-file: build
    nsl run --name {{NSL_APP}} ./target/release/als preview {{PREVIEW_FIXTURES}}/single.html --bind 127.0.0.1 --port NSL_PORT

# Single .md file — auto-bootstrapped single-chapter mdbook.
preview-md-file: build
    nsl run --name {{NSL_APP}} ./target/release/als preview {{PREVIEW_FIXTURES}}/single.md --bind 127.0.0.1 --port NSL_PORT

# Markdown folder with no book.toml — auto-bootstrapped (synth SUMMARY).
preview-md-dir: build
    nsl run --name {{NSL_APP}} ./target/release/als preview {{PREVIEW_FIXTURES}}/md-dir --bind 127.0.0.1 --port NSL_PORT

# Real mdbook project (book.toml + src/SUMMARY.md authored).
preview-mdbook: build
    nsl run --name {{NSL_APP}} ./target/release/als preview {{PREVIEW_FIXTURES}}/mdbook --bind 127.0.0.1 --port NSL_PORT

# .zip archive — rebuilt on demand from html-dir/ into target/.
preview-zip: build preview-zip-build
    nsl run --name {{NSL_APP}} ./target/release/als preview target/preview-fixtures/html-dir.zip --bind 127.0.0.1 --port NSL_PORT

# Pure-code directory — viewer shell + manifest + raw/ in a tempdir.
# The fixture intentionally also carries a README.md + page.html so the
# rendered/source toggle is exercised; `--kind code` is required to
# bypass auto-detect (which would otherwise route an md-containing
# folder through the markdown pipeline).
preview-code-dir: build
    nsl run --name {{NSL_APP}} ./target/release/als preview {{PREVIEW_FIXTURES}}/code-dir --kind code --bind 127.0.0.1 --port NSL_PORT

# Single recognised code file — viewer shell wrapping one source file.
preview-code-file: build
    nsl run --name {{NSL_APP}} ./target/release/als preview {{PREVIEW_FIXTURES}}/single.ts --bind 127.0.0.1 --port NSL_PORT

# Rebuild target/preview-fixtures/html-dir.zip from the html-dir fixture.
# Uses Python's stdlib zipfile module — no `zip` CLI required.
[private]
preview-zip-build:
    @mkdir -p target/preview-fixtures
    @cd {{PREVIEW_FIXTURES}}/html-dir && python3 -m zipfile -c "$PWD/../../../target/preview-fixtures/html-dir.zip" *
    @ls -lh target/preview-fixtures/html-dir.zip

# Offline format sweep — boots each fixture, curls /, asserts HTTP 200 + marker, kills it. Non-zero on first failure.
preview-sweep: build preview-zip-build
    bash {{PREVIEW_FIXTURES}}/sweep.sh

# ---------------------------------------------------------------------
# Cross-compilation via cargo-zigbuild
#
# Produces a portable static-linked musl binary suitable for any
# x86_64 Linux distribution. Requires `cargo-zigbuild` and `zig` on
# PATH (`cargo install cargo-zigbuild --locked` + a zig release
# unpacked anywhere on PATH).

# Dist-profile static-linked musl binary at target/x86_64-unknown-linux-musl/dist/als.
musl:
    cargo zigbuild --profile dist --bin als --target x86_64-unknown-linux-musl
    @ls -lh target/x86_64-unknown-linux-musl/dist/als

# Same as `musl` but for arm64 Linux.
musl-arm64:
    cargo zigbuild --profile dist --bin als --target aarch64-unknown-linux-musl
    @ls -lh target/aarch64-unknown-linux-musl/dist/als

# Side-by-side size report across the three release variants.
sizes: build musl
    @printf "  %-50s %s\n" "target/release/als (gnu, debug=line-tables)"  "$(du -h target/release/als | cut -f1)"
    @printf "  %-50s %s\n" "target/x86_64-unknown-linux-musl/dist/als"    "$(du -h target/x86_64-unknown-linux-musl/dist/als | cut -f1)"
