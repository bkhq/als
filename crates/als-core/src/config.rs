//! CLI configuration loader / saver.
//!
//! Resolves the on-disk path via `etcetera::choose_base_strategy()`
//! (`$XDG_CONFIG_HOME/als/config.toml` on Linux, `~/Library/Preferences/...`
//! on macOS via XDG, `%APPDATA%\als\config.toml` on Windows), parses the
//! TOML schema described in `docs/cli-spec.md` "Configuration file",
//! selects the active profile, and applies environment overrides.
//!
//! Resolution order:
//! * profile selection: `ALS_PROFILE` env > `--profile` flag > `[default]`
//! * value override: `ALS_API` / `ALS_TOKEN` always win over the section
//!
//! Tokens are wrapped in `secrecy::SecretString` so that `Debug` / `Display`
//! print `[REDACTED]` instead of the raw value.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use etcetera::{BaseStrategy, choose_base_strategy};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::Error;

const APP_DIR: &str = "als";
const FILE_NAME: &str = "config.toml";

/// Resolved CLI configuration. `token` is wrapped in `SecretString` so it
/// never appears in `Debug` or `Display` output.
#[derive(Debug, Clone)]
pub struct Config {
    pub api: Url,
    pub token: SecretString,
    pub active_profile: Option<String>,
    pub source_path: PathBuf,
}

/// On-disk TOML schema: one optional `[default]` block, zero or more
/// `[profile.<name>]` sections, and an auto-managed `[transport.<host:port>]`
/// table populated by the API client's transport probe.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Profile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profile: BTreeMap<String, Profile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub transport: BTreeMap<String, TransportEntry>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// One entry in `[transport.<host:port>]`. `protocol` is stored as a free
/// string so unknown values (typos, future additions) are tolerated by
/// the parser; consumers decide how to interpret it. Recognised values:
///
/// * `"h3"`  — pin to HTTP/3; the client still probes each session and
///   self-heals this entry to `"h2"` if h3 fails but h2 works.
/// * `"h2"`  — pin to HTTP/1.1+2; no probing.
/// * `"auto"` — probe every session, never write back. Use to escape a
///   stale pin without committing to a transport.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TransportEntry {
    pub protocol: String,
}

/// Returns the XDG-resolved config file path.
pub fn config_path() -> Result<PathBuf, Error> {
    let strategy = choose_base_strategy()
        .map_err(|e| Error::Config(format!("cannot resolve config directory: {e}")))?;
    Ok(strategy.config_dir().join(APP_DIR).join(FILE_NAME))
}

/// Load and resolve the active profile from the default XDG path,
/// reading overrides from the live process environment.
pub fn load(profile_arg: Option<&str>) -> Result<Config, Error> {
    let path = config_path()?;
    load_with_env(&path, profile_arg, live_env)
}

/// Atomic save: serialize to TOML, write a sibling `*.tmp` file with
/// owner-only permissions on Unix (`0o600`), then `rename` it onto the
/// target path so a partial write cannot replace the live config. If any
/// step after `create_dir_all` fails the tmp file is removed so a stale
/// plaintext-token file does not linger.
pub fn save(updated: &ConfigFile, path: &Path) -> Result<(), Error> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| Error::Config(format!("config path has no parent: {}", path.display())))?;
    let file_name = path.file_name().ok_or_else(|| {
        Error::Config(format!("config path has no file name: {}", path.display()))
    })?;
    std::fs::create_dir_all(parent)?;
    let serialized = toml::to_string_pretty(updated)
        .map_err(|e| Error::Config(format!("failed to serialize config: {e}")))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = parent.join(tmp_name);
    let result = write_then_rename(&tmp_path, path, &serialized);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

fn write_then_rename(tmp_path: &Path, final_path: &Path, contents: &str) -> Result<(), Error> {
    std::fs::write(tmp_path, contents)?;
    restrict_perms(tmp_path)?;
    std::fs::rename(tmp_path, final_path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_perms(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_perms(_path: &Path) -> Result<(), Error> {
    // Windows: rely on the user profile ACL inherited by %APPDATA%\als\.
    Ok(())
}

/// Read the raw [`ConfigFile`] at `path`, no profile resolution and no
/// env overrides. A missing file is *not* an error — the caller gets an
/// empty `ConfigFile` so it can mutate and save. Used by transport-cache
/// writers that need to round-trip the file without going through the
/// full profile-resolver pipeline.
pub fn read_raw(path: &Path) -> Result<ConfigFile, Error> {
    match std::fs::read_to_string(path) {
        Ok(content) => toml::from_str(&content)
            .map_err(|e| Error::Config(format!("failed to parse {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile::default()),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Look up the `protocol` for `host` from the `[transport.<host>]`
/// section of the config file at `path`. Returns `None` when the file
/// or the section is missing — callers treat that as "probe and decide".
/// I/O / parse errors are also swallowed to `None`: a broken config
/// must never block the API client.
pub fn read_transport(path: &Path, host: &str) -> Option<String> {
    let cfg = read_raw(path).ok()?;
    cfg.transport.get(host).map(|e| e.protocol.clone())
}

/// Render an obfuscated preview of a token: first 8 characters followed by
/// `**`. Used by `als auth status` and `als config list`.
pub fn mask_token(s: &SecretString) -> String {
    let head: String = s.expose_secret().chars().take(8).collect();
    let mut out = head;
    out.push_str("**");
    out
}

fn live_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

// Workspace-wide `forbid(unsafe_code)` plus Rust 2024's unsafe `env::set_var`
// makes scoped mutation of the real environment in tests impossible, so the
// resolver takes an injectable env lookup. Production callers pass the
// `live_env` reader above; tests pass a `HashMap`-backed closure.
pub(crate) fn load_with_env<F>(
    path: &Path,
    profile_arg: Option<&str>,
    env_fn: F,
) -> Result<Config, Error>
where
    F: Fn(&str) -> Option<String>,
{
    let content = std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::Config(format!(
            "config file not found at {}; run `als login` or `als config set` to create it",
            path.display()
        )),
        _ => Error::Io(e),
    })?;
    let parsed: ConfigFile = toml::from_str(&content)
        .map_err(|e| Error::Config(format!("failed to parse {}: {e}", path.display())))?;

    let active_profile = env_fn("ALS_PROFILE").or_else(|| profile_arg.map(str::to_owned));

    let section = match active_profile.as_deref() {
        Some(name) => parsed.profile.get(name).cloned().ok_or_else(|| {
            Error::Config(format!("profile `{name}` not found in {}", path.display()))
        })?,
        None => parsed.default.clone().unwrap_or_default(),
    };

    let api_str = env_fn("ALS_API")
        .or(section.api)
        .unwrap_or_else(|| crate::brand::DEFAULT_API_URL.to_owned());
    let token_str = env_fn("ALS_TOKEN").or(section.token).ok_or_else(|| {
        Error::Config("missing API token (set ALS_TOKEN or `token` in config)".into())
    })?;

    let api = Url::parse(&api_str)
        .map_err(|e| Error::Config(format!("invalid api URL `{api_str}`: {e}")))?;
    match api.scheme() {
        "http" | "https" => {}
        other => {
            return Err(Error::Config(format!(
                "api URL scheme must be http or https, got `{other}`"
            )));
        }
    }
    let token = SecretString::from(token_str);

    Ok(Config {
        api,
        token,
        active_profile,
        source_path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use secrecy::ExposeSecret;
    use tempfile::tempdir;

    use super::*;

    fn empty_env(_: &str) -> Option<String> {
        None
    }

    fn map_env(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |k| map.get(k).map(|v| (*v).to_owned())
    }

    fn sample_file() -> ConfigFile {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "staging".to_owned(),
            Profile {
                api: Some("https://staging.a.ls".into()),
                token: Some("tk_staging_token_value".into()),
            },
        );
        ConfigFile {
            default: Some(Profile {
                api: Some("https://api.a.ls".into()),
                token: Some("tk_default_token_value".into()),
            }),
            profile: profiles,
            ..Default::default()
        }
    }

    #[test]
    fn config_path_resolves_without_error() {
        let path = config_path().unwrap();
        assert!(path.ends_with(format!("{APP_DIR}/{FILE_NAME}")));
    }

    #[test]
    fn missing_file_returns_friendly_error() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope.toml");
        let err = load_with_env(&missing, None, empty_env).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
        let msg = err.to_string();
        assert!(msg.contains("config file not found"), "msg = {msg}");
        assert!(msg.contains(missing.display().to_string().as_str()));
    }

    #[test]
    fn invalid_toml_returns_config_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not = valid = toml").unwrap();
        let err = load_with_env(&path, None, empty_env).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
    }

    #[test]
    fn load_default_profile_when_no_arg() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&sample_file(), &path).unwrap();
        let cfg = load_with_env(&path, None, empty_env).unwrap();
        assert_eq!(cfg.api.as_str(), "https://api.a.ls/");
        assert_eq!(cfg.token.expose_secret(), "tk_default_token_value");
        assert!(cfg.active_profile.is_none());
        assert_eq!(cfg.source_path, path);
    }

    #[test]
    fn flag_selects_named_profile() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&sample_file(), &path).unwrap();
        let cfg = load_with_env(&path, Some("staging"), empty_env).unwrap();
        assert_eq!(cfg.api.as_str(), "https://staging.a.ls/");
        assert_eq!(cfg.token.expose_secret(), "tk_staging_token_value");
        assert_eq!(cfg.active_profile.as_deref(), Some("staging"));
    }

    #[test]
    fn env_profile_beats_flag() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut file = sample_file();
        file.profile.insert(
            "prod".to_owned(),
            Profile {
                api: Some("https://prod.a.ls".into()),
                token: Some("tk_prod_token_value".into()),
            },
        );
        save(&file, &path).unwrap();
        let env = map_env(HashMap::from([("ALS_PROFILE", "prod")]));
        let cfg = load_with_env(&path, Some("staging"), env).unwrap();
        assert_eq!(cfg.active_profile.as_deref(), Some("prod"));
        assert_eq!(cfg.token.expose_secret(), "tk_prod_token_value");
    }

    #[test]
    fn unknown_profile_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&sample_file(), &path).unwrap();
        let err = load_with_env(&path, Some("missing"), empty_env).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
        let msg = err.to_string();
        assert!(msg.contains("profile `missing`"), "msg = {msg}");
    }

    #[test]
    fn env_values_override_profile() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&sample_file(), &path).unwrap();
        let env = map_env(HashMap::from([
            ("ALS_API", "https://override.example.com"),
            ("ALS_TOKEN", "tk_override_value"),
        ]));
        let cfg = load_with_env(&path, None, env).unwrap();
        assert_eq!(cfg.api.as_str(), "https://override.example.com/");
        assert_eq!(cfg.token.expose_secret(), "tk_override_value");
    }

    #[test]
    fn env_values_supply_missing_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(
            &ConfigFile {
                default: None,
                profile: BTreeMap::new(),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        let env = map_env(HashMap::from([
            ("ALS_API", "https://only.example.com"),
            ("ALS_TOKEN", "tk_only"),
        ]));
        let cfg = load_with_env(&path, None, env).unwrap();
        assert_eq!(cfg.api.as_str(), "https://only.example.com/");
        assert_eq!(cfg.token.expose_secret(), "tk_only");
    }

    #[test]
    fn missing_api_falls_back_to_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(
            &ConfigFile {
                default: Some(Profile {
                    api: None,
                    token: Some("tk_x".into()),
                }),
                profile: BTreeMap::new(),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        let cfg = load_with_env(&path, None, empty_env).unwrap();
        let default_with_slash = format!("{}/", crate::brand::DEFAULT_API_URL);
        assert_eq!(cfg.api.as_str(), default_with_slash);
    }

    #[test]
    fn missing_token_errors_with_hint() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(
            &ConfigFile {
                default: Some(Profile {
                    api: Some("https://x.example.com".into()),
                    token: None,
                }),
                profile: BTreeMap::new(),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        let err = load_with_env(&path, None, empty_env).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
        let msg = err.to_string();
        assert!(msg.contains("ALS_TOKEN"), "msg = {msg}");
    }

    #[test]
    fn invalid_api_url_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(
            &ConfigFile {
                default: Some(Profile {
                    api: Some("not a url".into()),
                    token: Some("tk_x".into()),
                }),
                profile: BTreeMap::new(),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        let err = load_with_env(&path, None, empty_env).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
        let msg = err.to_string();
        assert!(msg.contains("invalid api URL"), "msg = {msg}");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let original = sample_file();
        save(&original, &path).unwrap();
        let cfg = load_with_env(&path, Some("staging"), empty_env).unwrap();
        assert_eq!(cfg.api.as_str(), "https://staging.a.ls/");
        assert_eq!(cfg.token.expose_secret(), "tk_staging_token_value");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[default]"));
        assert!(raw.contains("[profile.staging]"));
    }

    #[test]
    fn save_is_atomic_no_tmp_leftover() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&sample_file(), &path).unwrap();
        let tmp = dir.path().join("config.toml.tmp");
        assert!(!tmp.exists(), "tmp file should be renamed away");
        assert!(path.exists());
    }

    #[test]
    fn debug_does_not_leak_token() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&sample_file(), &path).unwrap();
        let cfg = load_with_env(&path, None, empty_env).unwrap();
        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("tk_default_token_value"),
            "Debug leaked token: {rendered}"
        );
        assert!(rendered.contains("REDACTED"), "Debug = {rendered}");
    }

    #[test]
    fn mask_token_keeps_first_eight_chars() {
        let secret = SecretString::from("abcdefghijklmno".to_owned());
        assert_eq!(mask_token(&secret), "abcdefgh**");
    }

    #[test]
    fn mask_token_short_token_is_padded() {
        let secret = SecretString::from("abc".to_owned());
        assert_eq!(mask_token(&secret), "abc**");
    }

    #[test]
    fn rejects_non_http_scheme() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(
            &ConfigFile {
                default: Some(Profile {
                    api: Some("file:///etc/passwd".into()),
                    token: Some("tk_x".into()),
                }),
                profile: BTreeMap::new(),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        let err = load_with_env(&path, None, empty_env).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
        let msg = err.to_string();
        assert!(msg.contains("http or https"), "msg = {msg}");
    }

    #[cfg(unix)]
    #[test]
    fn save_writes_owner_only_perms() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&sample_file(), &path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
    }

    #[test]
    fn save_rejects_bare_filename_with_no_parent() {
        let err = save(&sample_file(), Path::new("config.toml")).unwrap_err();
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
    }

    #[test]
    fn read_transport_returns_none_when_file_absent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.toml");
        assert!(read_transport(&path, "api.a.ls:443").is_none());
    }

    #[test]
    fn read_transport_returns_none_when_section_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&sample_file(), &path).unwrap();
        assert!(read_transport(&path, "api.a.ls:443").is_none());
    }

    #[test]
    fn read_transport_returns_user_written_protocol() {
        // The transport section is user-managed (the CLI no longer
        // writes it). Build a sample ConfigFile with an entry and
        // verify the reader surfaces the same string.
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut file = sample_file();
        file.transport.insert(
            "api.a.ls:443".into(),
            TransportEntry {
                protocol: "h3".into(),
            },
        );
        save(&file, &path).unwrap();
        assert_eq!(read_transport(&path, "api.a.ls:443").as_deref(), Some("h3"));
    }

    #[test]
    fn read_transport_tolerates_unknown_protocol_values() {
        // Parser must accept any string so a typo / future addition
        // does not break the CLI; the consumer treats unknown values
        // as "auto".
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut file = sample_file();
        file.transport.insert(
            "api.a.ls:443".into(),
            TransportEntry {
                protocol: "xyzzy".into(),
            },
        );
        save(&file, &path).unwrap();
        assert_eq!(
            read_transport(&path, "api.a.ls:443").as_deref(),
            Some("xyzzy")
        );
    }

    #[test]
    fn transport_section_round_trips_via_save() {
        // Confirm serde renders the quoted host key TOML expects so
        // hand-edited config files remain parsable after a `save`.
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut file = sample_file();
        file.transport.insert(
            "api.a.ls:443".into(),
            TransportEntry {
                protocol: "h3".into(),
            },
        );
        save(&file, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[transport.\"api.a.ls:443\"]"), "raw = {raw}");
        assert!(raw.contains("protocol = \"h3\""), "raw = {raw}");
    }

    #[test]
    fn read_raw_returns_empty_when_file_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.toml");
        let cfg = read_raw(&path).unwrap();
        assert!(cfg.default.is_none());
        assert!(cfg.profile.is_empty());
        assert!(cfg.transport.is_empty());
    }
}
