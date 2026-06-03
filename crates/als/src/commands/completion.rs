//! `als completion <shell>` — emit a static shell completion script.
//!
//! Pure-local command: no network, no config. The shell is validated by
//! `clap_complete::Shell` (a `ValueEnum`), so unknown values fail at clap
//! parse time with exit code 2 — this handler only runs for accepted
//! shells.
//!
//! Output mode is irrelevant: the spec mandates the raw script on stdout
//! regardless of `--quiet` / `--json`. The script is funneled through
//! `Output::quiet_line` (the same write path `config get` uses) to keep
//! `print_stdout` denied workspace-wide.

use als_core::{Error, Output};
use clap::CommandFactory;

use crate::cli::{Cli, CompletionArgs};

const BIN_NAME: &str = "als";

#[allow(clippy::unused_async)]
pub(crate) async fn run(out: &mut Output, args: CompletionArgs) -> Result<(), Error> {
    let script = generate_script(args.shell);
    out.quiet_line(script.trim_end_matches('\n'));
    Ok(())
}

fn generate_script(shell: clap_complete::Shell) -> String {
    let mut buf = Vec::new();
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, BIN_NAME, &mut buf);
    String::from_utf8(buf).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use clap_complete::Shell;

    use super::*;

    #[test]
    fn bash_script_mentions_binary_name() {
        let s = generate_script(Shell::Bash);
        assert!(s.contains(BIN_NAME), "bash script should reference 'als'");
        assert!(!s.is_empty());
    }

    #[test]
    fn zsh_script_mentions_binary_name() {
        let s = generate_script(Shell::Zsh);
        assert!(s.contains(BIN_NAME), "zsh script should reference 'als'");
        assert!(!s.is_empty());
    }

    #[test]
    fn fish_script_mentions_binary_name() {
        let s = generate_script(Shell::Fish);
        assert!(s.contains(BIN_NAME), "fish script should reference 'als'");
        assert!(!s.is_empty());
    }

    #[test]
    fn powershell_and_elvish_scripts_are_nonempty() {
        assert!(!generate_script(Shell::PowerShell).is_empty());
        assert!(!generate_script(Shell::Elvish).is_empty());
    }

    #[test]
    fn bash_script_lists_a_subcommand() {
        let s = generate_script(Shell::Bash);
        assert!(
            s.contains("config"),
            "completion should enumerate subcommands"
        );
    }
}
