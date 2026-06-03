//! `clap` derive structures. Globals (`--profile`, `--quiet`, `--json`) are
//! marked `global = true` so they accept the same value whether they appear
//! before or after the subcommand.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};

/// Footer for `--help`. Surfaces the built-in default API URL, the
/// marketing site, and the LLM usage cheatsheet hosted at
/// `https://a.ls/llms.txt` so both humans and AI agents can discover
/// the canonical entry points without scraping the README.
const HELP_FOOTER: &str = concat!(
    "Defaults:\n",
    "  api        ",
    "https://api.a.ls",
    "  (override with ALS_API or `als config set default.api <url>`)\n",
    "  homepage   ",
    "https://a.ls",
    "\n",
    "  AI usage   ",
    "https://a.ls/llms.txt",
);

#[derive(Parser, Debug)]
#[command(
    name = "als",
    version,
    about = "als — upload and publish static sites, markdown / mdbook books, and read-only code snippets",
    long_about = None,
    after_help = HELP_FOOTER,
    after_long_help = HELP_FOOTER,
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,

    /// Path to upload (folder, `.zip`, or single `.html` file).
    /// Used when no subcommand is given (the most frequent invocation).
    pub(crate) path: Option<PathBuf>,

    #[command(flatten)]
    pub(crate) deploy: DeployFlags,

    /// Named profile to load from the config file.
    #[arg(long, global = true)]
    pub(crate) profile: Option<String>,

    /// Suppress decoration; emit only essential output (e.g. the URL).
    #[arg(long, global = true)]
    pub(crate) quiet: bool,

    /// Emit machine-readable JSON instead of human text.
    #[arg(long, global = true)]
    pub(crate) json: bool,
}

/// Validator for `--name` arguments. Names live as filenames in
/// `sites/<id>_<name>.toml` and as URL-adjacent labels in the
/// console, so they're restricted to a kebab-case alphabet:
/// `[a-z0-9]([a-z0-9-]*[a-z0-9])?`. No uppercase, no underscores,
/// no Unicode, and no leading or trailing hyphen.
pub(crate) fn validate_name(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Err("name cannot be empty".into());
    }
    let valid_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    if !s.chars().all(valid_char) {
        return Err(format!(
            "name '{s}' has disallowed characters; allowed: lowercase a-z, digits 0-9, hyphen '-'"
        ));
    }
    if s.starts_with('-') || s.ends_with('-') {
        return Err(format!(
            "name '{s}' must start and end with a letter or digit, not '-'"
        ));
    }
    Ok(s.to_owned())
}

/// Flags accepted by the bare `als <path>` deploy invocation.
#[derive(Args, Debug)]
#[command(group(
    ArgGroup::new("deploy-pin").args(["name", "new_site"]).multiple(false),
))]
pub(crate) struct DeployFlags {
    /// Project name. Human-readable identity key, unique per user.
    /// Re-deploying with the same name pushes a new version to the
    /// existing site; the URL stays stable. Kebab-case lowercase
    /// only: `a-z`, `0-9`, `-` (no leading or trailing `-`).
    /// Mutually exclusive with `--new`.
    #[arg(long, value_parser = validate_name)]
    pub(crate) name: Option<String>,

    /// Enable access password. Pass `auto` to have the CLI generate a memorable one.
    #[arg(long)]
    pub(crate) pass: Option<String>,

    /// Site expiration (e.g. `24h`, `7d`, `never`, RFC 3339 timestamp).
    #[arg(long)]
    pub(crate) expires: Option<String>,

    /// Force a publish mode. Without this flag the CLI auto-detects:
    /// pure code → code, anything else → site.
    #[arg(long, value_enum)]
    pub(crate) kind: Option<KindFlag>,

    /// Force-create a fresh site, skipping both the local path pin and
    /// the server-side name dedup. Mutually exclusive with `--name`.
    #[arg(long = "new")]
    pub(crate) new_site: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum KindFlag {
    Site,
    Code,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// List owned sites with the per-user quota footer.
    List(ListArgs),
    /// Show a site's details, including the version history.
    /// Pass edit flags to change name / expiry / version / password.
    Site(SiteArgs),
    /// Read or write the local config file.
    Config(ConfigArgs),
    /// Remove a site (or every expired site with `--all-expired`).
    Rm(RmArgs),
    /// Drop a local path -> site binding from `~/.config/als/sites/<id>_<name>.toml`.
    Unpin(UnpinArgs),
    /// User-identity commands: `login`, `logout`, `revoke`, `status`.
    Auth(AuthArgs),
    /// Emit a shell completion script to stdout.
    Completion(CompletionArgs),
    /// Preview a path locally as it would appear on the server.
    Preview(PreviewArgs),
}

#[derive(Args, Debug)]
pub(crate) struct PreviewArgs {
    /// Path to preview (folder, `.zip`, `.html`, `.md`, mdbook root, or code).
    /// Defaults to the current directory.
    pub(crate) path: Option<PathBuf>,
    /// Address to bind. Defaults to localhost; pass `0.0.0.0` for LAN.
    #[arg(long, default_value = "127.0.0.1")]
    pub(crate) bind: String,
    /// Port to bind. `0` (default) lets the OS pick a free port.
    #[arg(long, default_value_t = 0)]
    pub(crate) port: u16,
    /// Force a preview mode (`site` or `code`). Defaults to auto-detect,
    /// matching the upload pipeline.
    #[arg(long, value_enum)]
    pub(crate) kind: Option<KindFlag>,
}

#[derive(Args, Debug)]
pub(crate) struct UnpinArgs {
    /// Canonical path, project name, or site id of the binding to drop.
    pub(crate) key: String,
}

#[derive(Args, Debug)]
#[command(group(
    ArgGroup::new("rm-target").args(["id_or_name", "all_expired"]).required(true).multiple(false),
))]
pub(crate) struct RmArgs {
    /// Site id or project name. Required unless `--all-expired` is set.
    pub(crate) id_or_name: Option<String>,
    /// Skip the interactive confirmation prompt.
    #[arg(short = 'y', long = "yes")]
    pub(crate) yes: bool,
    /// Remove every site whose state is `expired`.
    #[arg(long = "all-expired")]
    pub(crate) all_expired: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ListArgs {
    /// Lifecycle state to filter on. Filtering is applied client-side.
    #[arg(long, value_enum, default_value = "active")]
    pub(crate) state: StateArg,
    /// Show only sites with (`pass`) or without (`none`) password auth.
    /// Applied client-side after the server response.
    #[arg(long, value_enum)]
    pub(crate) auth: Option<AuthArg>,
    /// Maximum rows to request from the server (server cap: 200).
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum StateArg {
    Active,
    Expired,
    Released,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum AuthArg {
    None,
    Pass,
}

/// `als site <id|name> [flags...]`.
///
/// With no edit flag this is a detail view (site fields + version
/// history). Any edit flag turns it into an update; multiple edit flags
/// are combined into the smallest set of API calls (PATCH for name /
/// expiry, password POST for `--pass` / `--no-pass`, activate POST for
/// `--version`).
#[derive(Args, Debug)]
#[command(group(
    ArgGroup::new("site-pass").args(["pass", "no_pass"]).multiple(false),
))]
pub(crate) struct SiteArgs {
    /// Site id (10-char base32, e.g. `k7x2qm4j6p`) or project name.
    pub(crate) id_or_name: String,

    /// Rename the site (display only — URL is unaffected).
    /// Kebab-case lowercase: `a-z`, `0-9`, `-`.
    #[arg(long, value_parser = validate_name)]
    pub(crate) name: Option<String>,

    /// Set a new expiration: shorthand (`5m`, `24h`, `7d`, `1y`),
    /// `never`, or RFC 3339 timestamp.
    #[arg(long)]
    pub(crate) expires: Option<String>,

    /// Activate a previously deployed version (rollback / roll-forward).
    #[arg(long)]
    pub(crate) version: Option<String>,

    /// Set or replace the visitor password. Pass `auto` to have the
    /// server / CLI mint a memorable one.
    #[arg(long)]
    pub(crate) pass: Option<String>,

    /// Clear the visitor password (mutually exclusive with `--pass`).
    #[arg(long = "no-pass")]
    pub(crate) no_pass: bool,

    /// Skip the interactive confirmation prompt for destructive flags
    /// (`--no-pass`, `--version`).
    #[arg(short = 'y', long = "yes")]
    pub(crate) yes: bool,
}

impl SiteArgs {
    pub(crate) fn has_edit_flag(&self) -> bool {
        self.name.is_some()
            || self.expires.is_some()
            || self.version.is_some()
            || self.pass.is_some()
            || self.no_pass
    }
}

#[derive(Args, Debug)]
pub(crate) struct AuthArgs {
    #[command(subcommand)]
    pub(crate) action: AuthSub,
}

#[derive(Subcommand, Debug)]
pub(crate) enum AuthSub {
    /// Print a verification URL to copy into a browser; persist the issued token.
    Login(LoginArgs),
    /// Clear the local API token (does not revoke it server-side).
    Logout(LogoutArgs),
    /// Revoke the current API token server-side, then clear it locally.
    Revoke(RevokeArgs),
    /// Show the active profile's user, token, and quota.
    Status,
}

#[derive(Args, Debug)]
pub(crate) struct LoginArgs {
    /// Skip the terminal QR-code render of the verification URL. The
    /// URL itself is still printed for manual copy / phone scan.
    #[arg(long = "no-qr")]
    pub(crate) no_qr: bool,
    /// Persist the supplied token directly and skip the browser flow.
    /// Intended for CI usage where an operator already minted a token.
    #[arg(short = 'T', long = "token", value_name = "TOKEN")]
    pub(crate) token: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct LogoutArgs {}

#[derive(Args, Debug)]
pub(crate) struct RevokeArgs {
    /// Skip the interactive confirmation prompt.
    #[arg(short = 'y', long = "yes")]
    pub(crate) yes: bool,
}

#[derive(Args, Debug)]
pub(crate) struct CompletionArgs {
    /// Target shell. Unknown values trigger a clap parse error (exit 2).
    #[arg(value_enum)]
    pub(crate) shell: clap_complete::Shell,
}

#[derive(Args, Debug)]
pub(crate) struct ConfigArgs {
    #[command(subcommand)]
    pub(crate) action: ConfigSub,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ConfigSub {
    /// Print the value for a key (e.g. `default.api`).
    Get {
        /// Key formatted as `<profile>.<field>`, field ∈ {`api`, `token`}.
        key: String,
    },
    /// Set the value for a key.
    Set {
        /// Key formatted as `<profile>.<field>`, field ∈ {`api`, `token`}.
        key: String,
        /// New value (URL for `api`, opaque string for `token`).
        value: String,
    },
    /// Print the on-disk config file with tokens masked.
    List,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn bare_path_routes_to_deploy() {
        let cli = Cli::try_parse_from(["als", "./build"]).expect("parse");
        assert!(cli.command.is_none());
        assert_eq!(
            cli.path.as_deref(),
            Some(PathBuf::from("./build").as_path())
        );
        assert!(!cli.json);
        assert!(!cli.quiet);
        assert!(cli.deploy.pass.is_none());
        assert!(cli.deploy.expires.is_none());
        assert!(cli.deploy.kind.is_none());
    }

    #[test]
    fn kind_flag_parses_on_bare_deploy() {
        let cli = Cli::try_parse_from(["als", "./src", "--kind", "code"]).expect("parse");
        assert_eq!(cli.deploy.kind, Some(KindFlag::Code));

        let cli = Cli::try_parse_from(["als", "./src", "--kind", "site"]).expect("parse");
        assert_eq!(cli.deploy.kind, Some(KindFlag::Site));
    }

    #[test]
    fn kind_flag_parses_on_preview() {
        let cli =
            Cli::try_parse_from(["als", "preview", "./src", "--kind", "code"]).expect("parse");
        match cli.command {
            Some(Command::Preview(args)) => assert_eq!(args.kind, Some(KindFlag::Code)),
            _ => panic!("expected preview command"),
        }
    }

    #[test]
    fn site_command_parses_no_flags() {
        let cli = Cli::try_parse_from(["als", "site", "docs"]).expect("parse");
        match cli.command {
            Some(Command::Site(args)) => {
                assert_eq!(args.id_or_name, "docs");
                assert!(!args.has_edit_flag());
            }
            _ => panic!("expected site command"),
        }
    }

    #[test]
    fn site_command_accepts_combined_edit_flags() {
        let cli = Cli::try_parse_from([
            "als",
            "site",
            "k7x2qm4j6p",
            "--name",
            "newname",
            "--expires",
            "24h",
            "--pass",
            "auto",
        ])
        .expect("parse");
        match cli.command {
            Some(Command::Site(args)) => {
                assert_eq!(args.id_or_name, "k7x2qm4j6p");
                assert!(args.has_edit_flag());
                assert_eq!(args.name.as_deref(), Some("newname"));
                assert_eq!(args.expires.as_deref(), Some("24h"));
                assert_eq!(args.pass.as_deref(), Some("auto"));
                assert!(!args.no_pass);
            }
            _ => panic!("expected site command"),
        }
    }

    #[test]
    fn site_rejects_pass_with_no_pass() {
        let err = Cli::try_parse_from(["als", "site", "k7x2qm4j6p", "--pass", "x", "--no-pass"])
            .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn deploy_name_accepts_kebab_lowercase() {
        for good in ["myproj", "my-proj", "proj42", "fox042", "a"] {
            let cli = Cli::try_parse_from(["als", "./build", "--name", good]).expect("parse");
            assert_eq!(cli.deploy.name.as_deref(), Some(good));
        }
    }

    #[test]
    fn deploy_name_rejects_uppercase_punctuation_and_edges() {
        // `--name=<bad>` (equals form) so clap doesn't mistake a leading
        // hyphen for a separate option.
        for bad in [
            "MyProj",  // uppercase
            "My-Proj", // uppercase + hyphen
            "My Proj", // space
            "proj.v2", // dot
            "proj/v2", // slash
            "my_proj", // underscore
            "-myproj", // leading hyphen
            "myproj-", // trailing hyphen
            "",        // empty
        ] {
            let arg = format!("--name={bad}");
            let err = Cli::try_parse_from(["als", "./build", &arg]).unwrap_err();
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ValueValidation,
                "expected validation rejection for {bad:?}",
            );
        }
    }

    #[test]
    fn auth_status_parses() {
        let cli = Cli::try_parse_from(["als", "auth", "status"]).expect("parse");
        match cli.command {
            Some(Command::Auth(args)) => assert!(matches!(args.action, AuthSub::Status)),
            _ => panic!("expected auth command"),
        }
    }

    #[test]
    fn rm_accepts_id_or_name() {
        let cli = Cli::try_parse_from(["als", "rm", "docs"]).expect("parse");
        match cli.command {
            Some(Command::Rm(args)) => assert_eq!(args.id_or_name.as_deref(), Some("docs")),
            _ => panic!("expected rm"),
        }
    }

    #[test]
    fn rm_accepts_all_expired() {
        let cli = Cli::try_parse_from(["als", "rm", "--all-expired", "-y"]).expect("parse");
        match cli.command {
            Some(Command::Rm(args)) => {
                assert!(args.all_expired);
                assert!(args.yes);
                assert!(args.id_or_name.is_none());
            }
            _ => panic!("expected rm"),
        }
    }

    #[test]
    fn list_defaults_active() {
        let cli = Cli::try_parse_from(["als", "list"]).expect("parse");
        match cli.command {
            Some(Command::List(args)) => {
                assert_eq!(args.state, StateArg::Active);
                assert_eq!(args.limit, 50);
            }
            _ => panic!("expected list"),
        }
    }

    #[test]
    fn completion_accepts_known_shells() {
        for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
            let cli = Cli::try_parse_from(["als", "completion", shell]).expect("parse");
            assert!(matches!(cli.command, Some(Command::Completion(_))));
        }
    }

    #[test]
    fn completion_rejects_unknown_shell() {
        let err = Cli::try_parse_from(["als", "completion", "ksh"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
    }
}
