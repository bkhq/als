//! Per-command output sink that respects the user's chosen output style
//! (`Human` / `Quiet` / `Json`) and strips ANSI escape codes when the
//! underlying stream is not a TTY via [`anstream::AutoStream`].
//!
//! Production callers use [`Output::new`], which wraps the process
//! `stdout` / `stderr` in `AutoStream::auto`. Unit tests use
//! `Output::with_writers` to swap in `Vec<u8>`-backed sinks (wrapped in
//! `AutoStream::never`) so emitted text is deterministically free of ANSI
//! codes.
//!
//! `Output` is single-threaded — `AutoStream` does not guarantee `Send` /
//! `Sync` across all target configurations. Command handlers hold one
//! instance and pass it by `&mut`.

use std::io::Write;

use anstream::AutoStream;
use serde::Serialize;

use crate::error::Error;

/// Output formatting style requested by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Default interactive style: glyphs, color, tables, progress bars.
    Human,
    /// Suppress decoration; emit only essential lines (e.g. the URL).
    Quiet,
    /// Machine-readable JSON, one object per command.
    Json,
}

/// Stdout / stderr writer pair plus the active [`OutputMode`].
pub struct Output {
    mode: OutputMode,
    stdout: Box<dyn Write>,
    stderr: Box<dyn Write>,
}

impl Output {
    /// Build an `Output` targeting the process `stdout` / `stderr`.
    ///
    /// Each stream is wrapped in `AutoStream::auto`, so ANSI escape codes
    /// are stripped automatically when the descriptor is not a terminal.
    #[must_use]
    pub fn new(mode: OutputMode) -> Self {
        Self {
            mode,
            stdout: Box::new(AutoStream::auto(std::io::stdout())),
            stderr: Box::new(AutoStream::auto(std::io::stderr())),
        }
    }

    /// Test-only constructor: wrap the supplied writers in
    /// `AutoStream::never` so ANSI codes are always stripped, mirroring
    /// non-TTY production behavior.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_writers(
        mode: OutputMode,
        stdout: Box<dyn Write>,
        stderr: Box<dyn Write>,
    ) -> Self {
        Self {
            mode,
            stdout: Box::new(AutoStream::never(stdout)),
            stderr: Box::new(AutoStream::never(stderr)),
        }
    }

    /// Active output mode.
    #[must_use]
    pub fn mode(&self) -> OutputMode {
        self.mode
    }

    /// Run `body` to emit a human-formatted block on `stdout`.
    ///
    /// In `Quiet` and `Json` modes the closure is **not** invoked and
    /// nothing is written.
    pub fn human(&mut self, body: impl FnOnce(&mut dyn Write)) {
        if self.mode == OutputMode::Human {
            body(&mut self.stdout);
        }
    }

    /// Write `line` plus a trailing newline to `stdout`, in every mode.
    /// Used to emit the bare URL under `--quiet`.
    pub fn quiet_line(&mut self, line: &str) {
        let _ = writeln!(self.stdout, "{line}");
    }

    /// Serialize `value` to `stdout` as JSON terminated by exactly one
    /// trailing newline.
    ///
    /// # Errors
    /// Returns [`Error::Other`] if serialization fails, or [`Error::Io`]
    /// if writing the trailing newline fails.
    pub fn json<T: Serialize>(&mut self, value: &T) -> Result<(), Error> {
        serde_json::to_writer(&mut self.stdout, value)
            .map_err(|e| Error::Other(format!("json serialize: {e}")))?;
        self.stdout.write_all(b"\n").map_err(Error::Io)?;
        Ok(())
    }

    /// Emit an error to `stderr`. The shape depends on the active mode:
    ///
    /// - `Human`: `✗ kind: msg` in red, optionally followed by an
    ///   indented `Hint: …` line.
    /// - `Quiet`: `kind: msg` with no glyph, no color, no hint.
    /// - `Json` : `{"error":{"kind":..,"message":..,"hint":..}}` plus a
    ///   trailing newline.
    pub fn err(&mut self, kind: &str, msg: &str, hint: Option<&str>) {
        match self.mode {
            OutputMode::Human => {
                let _ = writeln!(self.stderr, "\x1b[31m✗\x1b[0m {kind}: {msg}");
                if let Some(h) = hint {
                    let _ = writeln!(self.stderr, "  Hint: {h}");
                }
            }
            OutputMode::Quiet => {
                let _ = writeln!(self.stderr, "{kind}: {msg}");
            }
            OutputMode::Json => {
                let envelope = serde_json::json!({
                    "error": {
                        "kind": kind,
                        "message": msg,
                        "hint": hint,
                    }
                });
                let _ = serde_json::to_writer(&mut self.stderr, &envelope);
                let _ = self.stderr.write_all(b"\n");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::io::{self, Write};
    use std::rc::Rc;

    use serde::Serialize;

    use super::{Output, OutputMode};

    type SharedBuf = Rc<RefCell<Vec<u8>>>;

    struct Sink(SharedBuf);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.borrow_mut().flush()
        }
    }

    fn make(mode: OutputMode) -> (Output, SharedBuf, SharedBuf) {
        let out: SharedBuf = SharedBuf::default();
        let err: SharedBuf = SharedBuf::default();
        let writer_out: Box<dyn Write> = Box::new(Sink(Rc::clone(&out)));
        let writer_err: Box<dyn Write> = Box::new(Sink(Rc::clone(&err)));
        let output = Output::with_writers(mode, writer_out, writer_err);
        (output, out, err)
    }

    fn read(buf: &SharedBuf) -> String {
        String::from_utf8(buf.borrow().clone()).unwrap_or_default()
    }

    #[test]
    fn mode_returns_constructed_mode() {
        let (out, _o, _e) = make(OutputMode::Json);
        assert_eq!(out.mode(), OutputMode::Json);
    }

    #[test]
    fn human_writes_in_human_mode() {
        let (mut out, stdout, _stderr) = make(OutputMode::Human);
        out.human(|w| {
            let _ = writeln!(w, "hello");
        });
        assert_eq!(read(&stdout), "hello\n");
    }

    #[test]
    fn human_is_noop_in_quiet_mode() {
        let (mut out, stdout, _stderr) = make(OutputMode::Quiet);
        let called = Cell::new(false);
        out.human(|w| {
            called.set(true);
            let _ = writeln!(w, "nope");
        });
        assert!(!called.get(), "closure must not run in Quiet mode");
        assert!(read(&stdout).is_empty());
    }

    #[test]
    fn human_is_noop_in_json_mode() {
        let (mut out, stdout, _stderr) = make(OutputMode::Json);
        let called = Cell::new(false);
        out.human(|w| {
            called.set(true);
            let _ = writeln!(w, "nope");
        });
        assert!(!called.get(), "closure must not run in Json mode");
        assert!(read(&stdout).is_empty());
    }

    #[test]
    fn quiet_line_writes_in_every_mode() {
        for mode in [OutputMode::Human, OutputMode::Quiet, OutputMode::Json] {
            let (mut out, stdout, _stderr) = make(mode);
            out.quiet_line("https://example.com");
            assert_eq!(read(&stdout), "https://example.com\n", "mode {mode:?}");
        }
    }

    #[test]
    fn json_writes_single_object_with_trailing_newline() {
        #[derive(Serialize)]
        struct Payload<'a> {
            id: &'a str,
            url: &'a str,
        }

        let (mut out, stdout, _stderr) = make(OutputMode::Json);
        out.json(&Payload {
            id: "k7x2qm4j6p",
            url: "https://k7x2qm4j6p.a.ls",
        })
        .unwrap();

        let raw = read(&stdout);
        assert!(raw.ends_with('\n'), "must end with newline");
        assert_eq!(raw.matches('\n').count(), 1, "exactly one newline");
        let trimmed = raw.trim_end_matches('\n');
        assert_eq!(
            trimmed,
            r#"{"id":"k7x2qm4j6p","url":"https://k7x2qm4j6p.a.ls"}"#
        );
    }

    #[test]
    fn err_human_writes_glyph_and_hint_with_ansi_stripped() {
        let (mut out, _stdout, stderr) = make(OutputMode::Human);
        out.err(
            "archive_too_large",
            "78MB exceeds 50MB limit",
            Some("split into smaller folders."),
        );
        let raw = read(&stderr);
        assert!(!raw.contains('\x1b'), "ANSI codes must be stripped");
        assert_eq!(
            raw,
            "✗ archive_too_large: 78MB exceeds 50MB limit\n  Hint: split into smaller folders.\n"
        );
    }

    #[test]
    fn err_human_without_hint_omits_hint_line() {
        let (mut out, _stdout, stderr) = make(OutputMode::Human);
        out.err("kind", "msg", None);
        assert_eq!(read(&stderr), "✗ kind: msg\n");
    }

    #[test]
    fn err_quiet_writes_plain_kind_message() {
        let (mut out, _stdout, stderr) = make(OutputMode::Quiet);
        out.err("kind", "msg", Some("ignored in quiet"));
        let raw = read(&stderr);
        assert!(!raw.contains('✗'));
        assert!(!raw.contains("Hint"));
        assert_eq!(raw, "kind: msg\n");
    }

    #[test]
    fn err_json_emits_envelope_with_hint() {
        let (mut out, _stdout, stderr) = make(OutputMode::Json);
        out.err("auth", "token invalid", Some("run `als login`"));
        let raw = read(&stderr);
        assert!(raw.ends_with('\n'));
        let trimmed = raw.trim_end_matches('\n');
        let value: serde_json::Value = serde_json::from_str(trimmed).unwrap();
        assert_eq!(value["error"]["kind"], "auth");
        assert_eq!(value["error"]["message"], "token invalid");
        assert_eq!(value["error"]["hint"], "run `als login`");
    }

    #[test]
    fn err_json_emits_null_hint_when_absent() {
        let (mut out, _stdout, stderr) = make(OutputMode::Json);
        out.err("server", "boom", None);
        let raw = read(&stderr);
        let trimmed = raw.trim_end_matches('\n');
        let value: serde_json::Value = serde_json::from_str(trimmed).unwrap();
        assert!(value["error"]["hint"].is_null());
    }

    #[test]
    fn ansi_stripped_when_writer_is_buffer() {
        // Regression guard: anstream::AutoStream::never wraps the buffer
        // and discards any escape sequences emitted by err()'s Human path.
        let (mut out, _stdout, stderr) = make(OutputMode::Human);
        out.err("kind", "msg", None);
        assert!(!read(&stderr).contains('\x1b'));
    }
}
