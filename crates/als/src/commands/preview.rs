//! `als preview [<path>]` — local HTTP preview of what `als <path>`
//! would publish. Reuses the same pre-pack pipeline (als-md for
//! Markdown / mdbook input; explicit handling of `.zip` and bare `.html`
//! files), then hands the resolved directory to `als-preview`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use als_core::{Error, Output};
use als_preview::PreviewOpts;
use tempfile::TempDir;

use crate::cli::{KindFlag, PreviewArgs};

/// Pre-resolve handle: keeps tempdirs / `als_md::Rendered` / code
/// bundles alive for the lifetime of the preview server.
#[allow(dead_code)]
enum PreviewGuard {
    Plain,
    Rendered(als_md::Rendered),
    Tmp(TempDir),
    Code(als_code::Bundle),
}

pub(crate) async fn run(out: &mut Output, args: PreviewArgs) -> Result<(), Error> {
    let path = args.path.unwrap_or_else(|| PathBuf::from("."));
    let (root, _guard) = resolve_preview_root(&path, args.kind)?;

    let server = als_preview::bind(
        root,
        &PreviewOpts {
            bind: args.bind.clone(),
            port: args.port,
        },
    )?;
    let url = server.url().to_owned();

    print_ready(out, &url, &args.bind);

    // tiny_http's request loop is synchronous; run it on a blocking
    // task so the runtime stays free to listen for SIGINT / SIGTERM.
    // On signal, `shutdown.shutdown()` wakes the loop, the worker
    // returns, and `_guard` drops on the way out of this function —
    // which is what actually deletes the tempdirs backing `.zip`,
    // single-file, and rendered-mdbook inputs.
    let shutdown = server.shutdown_handle();
    let worker = tokio::task::spawn_blocking(move || server.run());

    wait_for_shutdown_signal().await;
    shutdown.shutdown();

    worker
        .await
        .map_err(|e| Error::Other(format!("preview worker task failed: {e}")))?
}

fn print_ready(out: &mut Output, url: &str, bind: &str) {
    let url = url.to_owned();
    // A non-loopback bind exposes the preview root (which is the user's
    // source tree for plain-site / code modes) to anyone on the LAN.
    // Surface a red warning so the choice is visible at start time
    // rather than only inferred from the URL.
    let warning = if is_loopback_bind(bind) {
        None
    } else {
        Some(format!(
            "\x1b[31m⚠\x1b[0m  --bind {bind} exposes this preview to the LAN. \
             Anyone on this network can read every file under the preview root."
        ))
    };
    out.human(move |w| {
        let _ = writeln!(w, "Preview at {url}");
        if let Some(msg) = warning.as_deref() {
            let _ = writeln!(w, "{msg}");
        }
        let _ = writeln!(w, "  press Ctrl+C to stop");
    });
}

/// `127.0.0.1`, `::1`, and `localhost` resolve to loopback only and are
/// the only bind values that keep the preview private to this machine.
/// Everything else (including `0.0.0.0` / `::` / a public IP) is treated
/// as LAN-visible for warning purposes.
fn is_loopback_bind(bind: &str) -> bool {
    matches!(bind, "127.0.0.1" | "::1" | "localhost")
}

/// Block the current async task until SIGINT (Ctrl+C) or, on Unix,
/// SIGTERM is delivered. The future returns `()` either way — the
/// caller is responsible for triggering the actual server shutdown.
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        // SIGTERM stream registration can fail in unusual environments
        // (e.g. signal-fd exhaustion); fall back to ctrl_c-only in that
        // case so the common Ctrl+C path still works.
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn resolve_preview_root(
    path: &Path,
    kind: Option<KindFlag>,
) -> Result<(PathBuf, PreviewGuard), Error> {
    // Preview always uses the bundled-in viewer so it works without
    // network access and renders identically to what the matching
    // published version (CDN-served) would show.
    let viewer = als_code::ViewerSource::LocalEmbedded;

    if matches!(kind, Some(KindFlag::Code)) {
        let input = als_code::force_input(path)?;
        let bundle = als_code::bundle(input, viewer)?;
        return Ok((bundle.path().to_path_buf(), PreviewGuard::Code(bundle)));
    }

    // Auto-detect (or `--kind site`): markdown first, then code, then
    // fall through to the existing zip / html / dir handling.
    let force_site = matches!(kind, Some(KindFlag::Site));
    if !force_site && let Some(md_input) = als_md::classify(path)? {
        let rendered = als_md::render(md_input)?;
        return Ok((
            rendered.path().to_path_buf(),
            PreviewGuard::Rendered(rendered),
        ));
    }
    if !force_site && let Some(input) = als_code::classify(path)? {
        let bundle = als_code::bundle(input, viewer)?;
        return Ok((bundle.path().to_path_buf(), PreviewGuard::Code(bundle)));
    }

    let meta = fs::metadata(path)
        .map_err(|e| Error::Other(format!("path '{}' not accessible: {e}", path.display())))?;
    if meta.is_dir() {
        return Ok((path.to_path_buf(), PreviewGuard::Plain));
    }
    if !meta.is_file() {
        return Err(Error::Other(format!(
            "preview: '{}' is neither a regular file nor a directory",
            path.display()
        )));
    }

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("html") => {
            // Single-file `.html` is served as the tempdir root's
            // `index.html` so the banner-printed `/` resolves directly to
            // it, matching every other preview mode. Wrapping it under a
            // `<stem>/` subdir would force the reader to hand-fix the URL.
            let tmp = tempfile::tempdir()
                .map_err(|e| Error::Other(format!("create preview tempdir: {e}")))?;
            fs::copy(path, tmp.path().join("index.html"))
                .map_err(|e| Error::Other(format!("copy preview html: {e}")))?;
            let root = tmp.path().to_path_buf();
            Ok((root, PreviewGuard::Tmp(tmp)))
        }
        Some("zip") => {
            let tmp = tempfile::tempdir()
                .map_err(|e| Error::Other(format!("create preview tempdir: {e}")))?;
            extract_zip(path, tmp.path())?;
            let root = tmp.path().to_path_buf();
            Ok((root, PreviewGuard::Tmp(tmp)))
        }
        _ => Err(Error::Other(
            "preview: only directories, .zip, .html, .md, or mdbook roots accepted".into(),
        )),
    }
}

fn extract_zip(archive: &Path, out: &Path) -> Result<(), Error> {
    let file = fs::File::open(archive)
        .map_err(|e| Error::Other(format!("open zip '{}': {e}", archive.display())))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| Error::Other(format!("read zip: {e}")))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::Other(format!("zip entry {i}: {e}")))?;
        // `enclosed_name` rejects entries whose path tries to escape via
        // `..` or absolute components — the canonical zip-slip mitigation.
        let Some(rel) = entry.enclosed_name() else {
            return Err(Error::Other(format!(
                "zip entry refuses sandbox: {:?}",
                entry.name()
            )));
        };
        let dest = out.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&dest)
                .map_err(|e| Error::Other(format!("mkdir {}: {e}", dest.display())))?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::Other(format!("mkdir {}: {e}", parent.display())))?;
        }
        let mut writer = fs::File::create(&dest)
            .map_err(|e| Error::Other(format!("create {}: {e}", dest.display())))?;
        io::copy(&mut entry, &mut writer)
            .map_err(|e| Error::Other(format!("write {}: {e}", dest.display())))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::io::Cursor;

    use tempfile::tempdir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn resolve_directory_returns_path_directly() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.html"), b"<html/>").unwrap();
        let (root, _guard) = resolve_preview_root(dir.path(), None).unwrap();
        assert_eq!(root, dir.path());
    }

    #[test]
    fn resolve_single_html_serves_as_root_index() {
        let dir = tempdir().unwrap();
        let html = dir.path().join("report.html");
        fs::write(&html, b"<!doctype html><title>r</title>").unwrap();
        let (root, _guard) = resolve_preview_root(&html, None).unwrap();
        let index = root.join("index.html");
        assert!(index.is_file(), "{index:?} should exist");
    }

    #[test]
    fn resolve_zip_extracts_to_tempdir() {
        let src = tempdir().unwrap();
        let zip_path = src.path().join("payload.zip");
        let mut buf = Cursor::new(Vec::<u8>::new());
        {
            let mut w = ZipWriter::new(&mut buf);
            w.start_file("index.html", SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut w, b"<html/>").unwrap();
            w.start_file("nested/a.txt", SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut w, b"alpha").unwrap();
            w.finish().unwrap();
        }
        fs::write(&zip_path, buf.into_inner()).unwrap();

        let (root, _guard) = resolve_preview_root(&zip_path, None).unwrap();
        assert!(root.join("index.html").is_file());
        assert!(root.join("nested/a.txt").is_file());
    }

    #[test]
    fn is_loopback_bind_recognises_local_only_addresses() {
        assert!(is_loopback_bind("127.0.0.1"));
        assert!(is_loopback_bind("::1"));
        assert!(is_loopback_bind("localhost"));
        assert!(!is_loopback_bind("0.0.0.0"));
        assert!(!is_loopback_bind("::"));
        assert!(!is_loopback_bind("192.168.1.10"));
    }

    #[test]
    fn resolve_rejects_unsupported_single_file() {
        let dir = tempdir().unwrap();
        let txt = dir.path().join("note.txt");
        fs::write(&txt, b"nope").unwrap();
        let Err(err) = resolve_preview_root(&txt, None) else {
            panic!("expected resolve to reject .txt");
        };
        match err {
            Error::Other(msg) => assert!(msg.contains("only directories")),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
