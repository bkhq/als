//! Static-file HTTP server for `als preview <path>`.
//!
//! Pure-Rust GET-only server backed by `tiny_http`. The caller hands in
//! a fully resolved directory; this crate knows nothing about Markdown,
//! mdbook, zip extraction, or any other input shape — that orchestration
//! lives in the binary (`crates/als/src/commands/preview.rs`).
//!
//! Lifecycle:
//! ```ignore
//! let server = als_preview::bind(root, opts)?;
//! let shutdown = server.shutdown_handle();
//! // ... install a signal handler that calls `shutdown.shutdown()` ...
//! server.run()?;  // blocks until shutdown is signalled
//! ```

mod mime;
mod path_resolve;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use als_core::Error;
use tiny_http::{Header, Method, Response, Server, StatusCode};

/// Bind options for the preview server.
#[derive(Debug, Clone)]
pub struct PreviewOpts {
    /// Address to listen on. `127.0.0.1` for local-only (default);
    /// `0.0.0.0` exposes the preview to the LAN.
    pub bind: String,
    /// Port. `0` lets the OS pick a free port.
    pub port: u16,
}

impl Default for PreviewOpts {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: 0,
        }
    }
}

/// A bound, not-yet-running preview server. The URL is known once
/// [`bind`] returns; the caller can log it before calling
/// [`PreviewServer::run`] (which blocks).
pub struct PreviewServer {
    inner: Arc<Server>,
    root: PathBuf,
    url: String,
}

impl PreviewServer {
    /// HTTP URL the server is listening on (e.g. `http://127.0.0.1:54123/`).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Hand out a cloneable handle that wakes [`PreviewServer::run`] from
    /// another thread. The binary obtains one before launching the
    /// request loop and calls [`ShutdownHandle::shutdown`] on
    /// SIGINT / SIGTERM so the loop returns cleanly and the caller's
    /// tempdir guards get a chance to drop.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            server: self.inner.clone(),
        }
    }

    /// Block on the request loop until [`ShutdownHandle::shutdown`] is
    /// called from another thread (typically a signal handler).
    pub fn run(self) -> Result<(), Error> {
        for request in self.inner.incoming_requests() {
            handle_request(request, &self.root);
        }
        Ok(())
    }
}

/// Cross-thread handle that interrupts [`PreviewServer::run`].
/// Inexpensive to clone — internally just an `Arc` to the `tiny_http`
/// server.
#[derive(Clone)]
pub struct ShutdownHandle {
    server: Arc<Server>,
}

impl ShutdownHandle {
    /// Wake the request loop. The next iteration of
    /// `incoming_requests()` returns `None` and [`PreviewServer::run`]
    /// returns `Ok(())`. Safe to call multiple times.
    pub fn shutdown(&self) {
        self.server.unblock();
    }
}

/// Bind a preview server over `root`. Returns once the TCP socket is
/// listening; the caller chooses when to start the request loop.
pub fn bind(root: PathBuf, opts: &PreviewOpts) -> Result<PreviewServer, Error> {
    if !root.is_dir() {
        return Err(Error::Other(format!(
            "preview root '{}' is not a directory",
            root.display()
        )));
    }
    let bind_str = format!("{}:{}", opts.bind, opts.port);
    let server = Server::http(&bind_str)
        .map_err(|e| Error::Other(format!("preview bind {bind_str} failed: {e}")))?;
    let addr = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| Error::Other("preview server bound to non-IP address".into()))?;
    let url = format!("http://{addr}/");
    Ok(PreviewServer {
        inner: Arc::new(server),
        root,
        url,
    })
}

fn handle_request(request: tiny_http::Request, root: &std::path::Path) {
    if !matches!(request.method(), Method::Get | Method::Head) {
        let _ = request.respond(text_response(
            StatusCode(405),
            "method not allowed\n",
            "text/plain; charset=utf-8",
        ));
        return;
    }
    let is_head = matches!(request.method(), Method::Head);
    let url = request.url().to_owned();
    let Some(resolved) = path_resolve::resolve(root, &url) else {
        let _ = request.respond(text_response(
            StatusCode(404),
            "not found\n",
            "text/plain; charset=utf-8",
        ));
        return;
    };
    // HEAD must not carry a body. tiny_http does not strip the body
    // automatically, so short-circuit before touching the file and
    // return just the headers needed for content sniffing. We also
    // skip the disk read entirely — HEAD is cheap and we don't want
    // to surface a 404 just because metadata-only callers raced an
    // unlink that GETs would have caught earlier.
    if is_head {
        let mime = mime::for_path(&resolved);
        let resp = Response::empty(StatusCode(200)).with_header(content_type_header(mime));
        let _ = request.respond(resp);
        return;
    }
    match fs::read(&resolved) {
        Ok(bytes) => {
            let mime = mime::for_path(&resolved);
            let resp = Response::from_data(bytes).with_header(content_type_header(mime));
            let _ = request.respond(resp);
        }
        Err(_) => {
            let _ = request.respond(text_response(
                StatusCode(404),
                "not found\n",
                "text/plain; charset=utf-8",
            ));
        }
    }
}

fn text_response(
    status: StatusCode,
    body: &'static str,
    mime: &'static str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(body.as_bytes().to_vec())
        .with_status_code(status)
        .with_header(content_type_header(mime))
}

/// Build a `Content-Type` header from a static MIME string. Every MIME
/// value in the lookup table (and every literal passed by
/// `text_response`) is ASCII, so `Header::from_bytes` is infallible in
/// practice; the `expect` documents that invariant.
#[allow(clippy::expect_used)]
fn content_type_header(mime: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], mime.as_bytes())
        .expect("Content-Type header from ASCII mime literal always parses")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn bind_rejects_non_directory_root() {
        let Err(err) = bind(PathBuf::from("/does/not/exist"), &PreviewOpts::default()) else {
            panic!("expected bind to fail on missing dir");
        };
        match err {
            Error::Other(msg) => assert!(msg.contains("not a directory")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bind_succeeds_and_url_has_random_port_when_port_zero() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();
        let Ok(server) = bind(dir.path().to_path_buf(), &PreviewOpts::default()) else {
            panic!("bind should succeed on a valid directory");
        };
        assert!(server.url().starts_with("http://127.0.0.1:"));
        // 0 means OS-assigned, which is always a non-zero ephemeral port.
        assert!(!server.url().ends_with(":0/"));
    }

    #[test]
    fn shutdown_handle_unblocks_request_loop() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.html"), b"<html></html>").unwrap();
        let server = bind(dir.path().to_path_buf(), &PreviewOpts::default()).unwrap();
        let shutdown = server.shutdown_handle();
        let worker = std::thread::spawn(move || server.run());
        // Give the request loop a moment to enter `incoming_requests`.
        std::thread::sleep(std::time::Duration::from_millis(50));
        shutdown.shutdown();
        let start = std::time::Instant::now();
        let res = worker.join().expect("worker should not panic");
        assert!(res.is_ok(), "server.run() should return Ok after shutdown");
        // tiny_http's `unblock` should wake the loop near-instantly.
        // Give a generous bound to avoid flake on a loaded CI.
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "shutdown should be near-instant, took {:?}",
            start.elapsed()
        );
    }
}
