//! `als config get|set|list` — local-only TOML manipulation. No network.

use als_core::{
    Error, Output, OutputMode,
    config::{ConfigFile, Profile, config_path, save},
    mask_token,
};
use secrecy::SecretString;
use serde_json::json;
use url::Url;

use crate::cli::{ConfigArgs, ConfigSub};

// Every command handler exposes the same `async fn run` shape even
// when the body is synchronous (this one does only local file I/O).
#[allow(clippy::unused_async)]
pub(crate) async fn run(out: &mut Output, args: ConfigArgs) -> Result<(), Error> {
    match args.action {
        ConfigSub::Get { key } => get(out, &key),
        ConfigSub::Set { key, value } => set(out, &key, &value),
        ConfigSub::List => list(out),
    }
}

/// Read and parse the on-disk config file. Treats `NotFound` as an empty
/// `ConfigFile` so that `set` can bootstrap from scratch.
fn load_file() -> Result<ConfigFile, Error> {
    let path = config_path()?;
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s)
            .map_err(|e| Error::Config(format!("failed to parse {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile::default()),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Split `<profile>.<field>` into its two halves, validating that the field
/// is one of the two writable keys.
fn parse_key(key: &str) -> Result<(&str, &str), Error> {
    let (profile, field) = key
        .split_once('.')
        .ok_or_else(|| Error::Config(format!("unknown key '{key}'")))?;
    if profile.is_empty() || !matches!(field, "api" | "token") {
        return Err(Error::Config(format!("unknown key '{key}'")));
    }
    Ok((profile, field))
}

/// `default` resolves to the top-level `[default]` block; any other name to
/// the matching `[profile.<name>]` section.
fn section_of<'a>(file: &'a ConfigFile, name: &str) -> Option<&'a Profile> {
    if name == "default" {
        file.default.as_ref()
    } else {
        file.profile.get(name)
    }
}

fn get(out: &mut Output, key: &str) -> Result<(), Error> {
    let (profile_name, field) = parse_key(key)?;
    let file = load_file()?;
    let section = section_of(&file, profile_name)
        .ok_or_else(|| Error::Config(format!("unknown key '{key}'")))?;
    let raw = if field == "api" {
        section.api.as_deref()
    } else {
        section.token.as_deref()
    }
    .ok_or_else(|| Error::Config(format!("unknown key '{key}'")))?;

    let value = if field == "token" {
        mask_token(&SecretString::from(raw.to_owned()))
    } else {
        raw.to_owned()
    };

    if out.mode() == OutputMode::Json {
        out.json(&json!({ "key": key, "value": value }))?;
    } else {
        out.quiet_line(&value);
    }
    Ok(())
}

fn validate_api(value: &str) -> Result<(), Error> {
    let url =
        Url::parse(value).map_err(|e| Error::Config(format!("invalid api URL `{value}`: {e}")))?;
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(Error::Config(format!(
            "api URL scheme must be http or https, got `{other}`"
        ))),
    }
}

fn set(out: &mut Output, key: &str, value: &str) -> Result<(), Error> {
    let (profile_name, field) = parse_key(key)?;
    if field == "api" {
        validate_api(value)?;
    }
    let mut file = load_file()?;
    let target: &mut Profile = if profile_name == "default" {
        file.default.get_or_insert_with(Profile::default)
    } else {
        file.profile.entry(profile_name.to_owned()).or_default()
    };
    if field == "api" {
        target.api = Some(value.to_owned());
    } else {
        target.token = Some(value.to_owned());
    }
    let path = config_path()?;
    save(&file, &path)?;

    if out.mode() == OutputMode::Json {
        out.json(&json!({ "updated": key }))?;
    }
    Ok(())
}

fn list(out: &mut Output) -> Result<(), Error> {
    let mut file = load_file()?;
    mask_tokens(&mut file);
    match out.mode() {
        OutputMode::Json => out.json(&file)?,
        OutputMode::Human | OutputMode::Quiet => {
            for line in human_lines(&file) {
                out.quiet_line(&line);
            }
        }
    }
    Ok(())
}

fn mask_tokens(file: &mut ConfigFile) {
    if let Some(p) = file.default.as_mut() {
        mask_in_place(p);
    }
    for p in file.profile.values_mut() {
        mask_in_place(p);
    }
}

fn mask_in_place(p: &mut Profile) {
    if let Some(raw) = p.token.take() {
        p.token = Some(mask_token(&SecretString::from(raw)));
    }
}

fn human_lines(file: &ConfigFile) -> Vec<String> {
    let mut sections: Vec<(String, &Profile)> = Vec::new();
    if let Some(p) = file.default.as_ref() {
        sections.push(("[default]".to_owned(), p));
    }
    for (name, p) in &file.profile {
        sections.push((format!("[profile.{name}]"), p));
    }

    let mut lines = Vec::new();
    for (i, (header, profile)) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.push(header.clone());
        push_kv(&mut lines, profile);
    }
    lines
}

fn push_kv(lines: &mut Vec<String>, p: &Profile) {
    if let Some(api) = p.api.as_ref() {
        lines.push(format!("api    = {api}"));
    }
    if let Some(token) = p.token.as_ref() {
        lines.push(format!("token  = {token}"));
    }
}
