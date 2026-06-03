//! Built-in brand URLs.
//!
//! The CLI ships with `https://api.a.ls` as the default API endpoint and
//! `https://a.ls` as the marketing / web entry point. Users can override
//! either by writing to `~/.config/als/config.toml`, setting `ALS_API`,
//! or running `als config set default.api <url>` — but for the common
//! case of using the public service no configuration is needed.

/// Default API base URL used by `als auth login` when no profile-level
/// `api` value is present (env, config file, or otherwise). Normalised
/// by `als_api::client::Client::new` to `<this>/api/`.
pub const DEFAULT_API_URL: &str = "https://api.a.ls";

/// Marketing site / web entry point. Surfaced in `--help` and used by
/// hint URLs in error messages.
pub const HOMEPAGE_URL: &str = "https://a.ls";

/// Self-describing usage cheatsheet for LLM agents. Hosted at the
/// marketing site root so agents can fetch it without auth or version
/// pinning. Surfaced in `--help` so curious humans can read it too.
pub const LLMS_TXT_URL: &str = "https://a.ls/llms.txt";
