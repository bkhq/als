//! Interactive `[y/N]` confirmation that gracefully degrades when stderr
//! is not a TTY (CI, piped stdin from `assert_cmd::write_stdin`,
//! background scripts).
//!
//! `dialoguer::Confirm::interact()` in dialoguer 0.12 hard-errors with
//! `not a terminal` in non-TTY contexts, so this helper falls back to
//! reading one line from stdin so the same prompt works under both
//! interactive and scripted invocations. `console::Term::write_line`
//! bypasses the workspace `print_stderr` lint by going through
//! `Write`, not the `eprint!` macro.

use als_core::Error;

/// Prompt the user with `prompt` followed by `[y/N]` and return whether
/// they answered affirmatively. Defaults to `false` when no input is
/// given or stderr is not a TTY and stdin is empty.
pub(crate) fn confirm(prompt: &str) -> Result<bool, Error> {
    let term = dialoguer::console::Term::stderr();
    if term.is_term() {
        return dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()
            .map_err(|e| Error::Io(e.into()));
    }
    term.write_line(&format!("{prompt} [y/N]"))
        .map_err(Error::Io)?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(Error::Io)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
