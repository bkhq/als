# als-preview

Pure-Rust static-file HTTP server for `als preview <path>`. GET / HEAD
only, MIME-table-driven, percent-decoded path resolve, graceful
shutdown via `tiny_http::Server::unblock`.

The caller hands in a fully resolved root directory; this crate
knows nothing about Markdown, mdbook, zip extraction, or any other
input shape. Format dispatch lives in
`crates/als/src/commands/preview.rs`.

## Lifecycle

```rust
use als_preview::{bind, PreviewOpts, ShutdownHandle};

let server = bind(root_dir, &PreviewOpts {
    bind: "127.0.0.1".into(),
    port: 0, // OS-assigned
})?;
println!("Preview at {}", server.url());

let shutdown: ShutdownHandle = server.shutdown_handle();
// hand `shutdown` to a signal handler; it is Clone + Send + Sync.

server.run()?;  // blocks until ShutdownHandle::shutdown() is called
```

`bind` returns once the TCP socket is listening, so the caller can
log the URL before starting the request loop. `run` blocks on
`tiny_http`'s `incoming_requests()` iterator; `shutdown_handle()`
hands out a cloneable token whose `shutdown()` method calls
`Server::unblock`, ending the iterator cleanly.

The binary wires SIGINT and (Unix) SIGTERM through this handle so
the caller's tempdir guards (`TempDir`, `als_md::Rendered`) drop
via the usual stack unwind when `run` returns.

## Request handling

| Method     | Behaviour                                            |
|------------|------------------------------------------------------|
| `GET`/`HEAD` | Resolve, return file with MIME from `mime::for_path`. |
| Other      | `405 method not allowed`                             |

Path resolution (`path_resolve::resolve`):

1. Percent-decode the URL path.
2. Strip the leading `/`.
3. Join under `root` and canonicalise.
4. If the canonical path is not within `root`, return `None` (404).
   This is the zip-slip mitigation.
5. If the resolved path is a directory, look for `index.html` inside
   it.

Returns 404 on any miss; never lists directories.

## MIME table

`mime::for_path` matches by extension against a static table covering
HTML, CSS, JS, JSON, XML, plain text, common image formats (PNG /
JPG / GIF / SVG / WebP / AVIF), font formats (WOFF2 / WOFF / TTF /
OTF), audio / video, and a few document formats (PDF). Unknown
extensions fall back to `application/octet-stream`.

## Tests

```text
bind_rejects_non_directory_root
bind_succeeds_and_url_has_random_port_when_port_zero
shutdown_handle_unblocks_request_loop
path_resolve::tests::* (4)
mime::tests::* (n)
```

`shutdown_handle_unblocks_request_loop` pins the SIGINT-safety
contract: spawn the request loop on a worker thread, sleep
briefly, call `shutdown`, expect the worker to return under 2
seconds. Run with `cargo test -p als-preview --lib`.

## Dependencies

- `tiny_http` (default features off) — the HTTP runtime. Sync, no
  TLS, perfect for a localhost preview.
- `als-core` — `Error` enum.

No async, no tokio, no hyper. The binary side parks tokio while the
request loop runs on a `spawn_blocking` task.
