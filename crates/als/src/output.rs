//! Crate-local helpers on top of `als_core::output`. Workspace-deny on
//! `print_stdout` / `print_stderr` stays in force everywhere else; only this
//! module routes errors through the shared sink.

use als_core::{Error, Output};

/// Translate a `als_core::Error` into a structured `(kind, message, hint)`
/// triple and emit it through `Output::err`, which renders per the active
/// `OutputMode`.
pub(crate) fn report_error(out: &mut Output, err: &Error) {
    match err {
        Error::Auth => out.err(
            "auth_required",
            "your token is invalid or revoked",
            Some("run `als login` again"),
        ),
        Error::NotFound { what, hint } => {
            out.err("not_found", &format!("{what} not found"), hint.as_deref());
        }
        Error::Quota => out.err(
            "quota_exceeded",
            "your quota is full",
            Some("see https://a.ls/settings"),
        ),
        Error::RateLimited(s) => out.err("rate_limited", &format!("retry after {s}s"), None),
        Error::Network(m) => out.err("network", m, None),
        Error::Server(m) => out.err("server_error", m, None),
        Error::Config(m) => out.err("config", m, None),
        Error::Io(e) => out.err("io", &e.to_string(), None),
        // `als auth login` renders the spec-mandated `✗ login_timeout: …`
        // line directly when the pairing poll exhausts its window and
        // returns this exact sentinel for exit code 1; skip re-rendering.
        Error::Other(m) if m == "login_timeout" => {}
        // Same pattern for `access_denied` — login() prints
        // `✗ login_denied: …` itself and returns this sentinel.
        Error::Other(m) if m == "login_denied" => {}
        // 413 `archive_too_large` arrives as `Error::Other("archive too large")`
        // via the wire-error catch-all in `From<ApiError> for als_core::Error`.
        // Render the dedicated `kind` + Hint per `cli-spec.md` "Error handling".
        Error::Other(m) if m == "archive too large" => out.err(
            "archive_too_large",
            "uploaded archive exceeds the server size limit",
            Some("split into smaller folders or trim via als.toml `exclude`"),
        ),
        // Code-mode limit / shape errors carry their stable kind prefix in
        // the leading `code_*:` segment. Split that off and route to a
        // user-friendly hint per `cli-spec.md`.
        Error::Other(m) if m.starts_with("code_") => report_code(out, m),
        Error::Other(m) => out.err("error", m, None),
    }
}

/// Render a `code_*` error. The originating string is shaped
/// `"<kind>: <detail>"`; we keep the kind for the structured channel
/// and append a hint that points users at the lever they can pull.
fn report_code(out: &mut Output, raw: &str) {
    let (kind, detail) = raw
        .split_once(':')
        .map_or((raw, raw), |(k, d)| (k, d.trim()));
    let hint: Option<&str> = match kind {
        "code_too_many_files" => Some("add patterns to als.toml `exclude` or drop unused files"),
        "code_file_too_large" => Some("trim or split the file"),
        "code_total_too_large" => Some("trim the input or use --kind site"),
        "code_depth_exceeded" => Some("flatten the layout or exclude deep paths via als.toml"),
        "code_no_files" => Some("check als.toml `exclude` or supply a different path"),
        "code_unsupported_input" => Some("--kind code needs a code file or directory of code"),
        "code_default_file_missing" => {
            Some("update als.toml `default_file` to point at a bundled file")
        }
        _ => None,
    };
    out.err(kind, detail, hint);
}
