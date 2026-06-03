//! `als <path>` — pack a folder / zip / single HTML file and POST it to
//! `/api/deploy`, then render the response per the active `OutputMode`.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use als_api::{ApiError, Client, DeployRequest, DeployResponse};
use als_core::sites::{self, SiteEntry};
use als_core::{Error, Output, OutputMode, config, parse_duration};
use als_pack::{classify, pack};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rand::Rng;
use serde::Serialize;

use crate::cli::{DeployFlags, KindFlag};

/// Pre-pack handle: keeps either a `als_md::Rendered` mdbook output or
/// a `als_code::Bundle` tempdir alive until the upload finishes.
/// The variant payload is held only for its `Drop` (tempdir cleanup).
enum InputGuard {
    Plain,
    Rendered(#[allow(dead_code)] als_md::Rendered),
    Code(#[allow(dead_code)] als_code::Bundle),
}

/// Outcome of `resolve_kind`: what kind of payload to assemble.
enum Resolved<'a> {
    Site,
    Code(als_code::CodeInput<'a>),
}
use crate::commands::list::{current_iso, format_expires};

const PROGRESS_TICK_MS: u64 = 100;
// 4 groups × 3 chars over a 30-symbol alphabet ≈ 59 bits of entropy,
// roughly a billion times stronger than the prior 3×2 layout (~30 bits)
// while still typeable in one breath. Visitor-side rate limiting on the
// server is the second line of defence, but the client picks the
// brute-force floor.
const AUTO_PASS_GROUPS: usize = 4;
const AUTO_PASS_GROUP_LEN: usize = 3;
// Memorable alphabet: lowercase letters and digits with visually ambiguous
// characters removed (i/l/o, 0/1) so a human can re-type a generated password.
const AUTO_PASS_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// JSON output: the server `DeployResponse` plus the locally-generated
/// password (when `--pass auto` was used). The password never appears in
/// the API response, so it would otherwise be lost to script consumers.
#[derive(Serialize)]
struct DeployJson<'a> {
    #[serde(flatten)]
    response: &'a DeployResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
}

pub(crate) async fn run(
    out: &mut Output,
    profile: Option<&str>,
    path: &Path,
    flags: DeployFlags,
) -> Result<(), Error> {
    let cfg = config::load(profile)?;

    let pin_ctx = resolve_pin(path, &flags)?;

    // Source fingerprint over the original `<path>`. Used both for the
    // pre-pipeline skip check and to bookkeep the new pin entry on
    // success.
    let content_hash = als_pack::digest(path)?;

    // No-op skip: if the user invoked `als <path>` with no metadata
    // flags and a prior pin exists for this exact path with a matching
    // fingerprint, the upload would be byte-identical and produce no
    // change visible to a visitor. Short-circuit before any rendering /
    // packing / network so re-running the same command stays cheap.
    if is_metadata_only(&flags)
        && let Some(prior) = pin_ctx.prior.as_ref()
        && prior.last_content_hash.as_deref() == Some(content_hash.as_str())
    {
        return render_no_change(out, prior);
    }

    // Route the path. Markdown / mdbook still pre-renders into a tempdir
    // and uploads that; pure code goes through `als-code`; anything
    // else stays on the plain als-pack path. Each guard variant owns
    // whatever tempdir its branch produced.
    let (pack_root, _guard) = match resolve_kind(&flags, path)? {
        Resolved::Code(input) => {
            // Uploads point HTML at the pinned CDN viewer so the published
            // site is fully cacheable and version-stable.
            let bundle = als_code::bundle(input, als_code::ViewerSource::Cdn)?;
            (bundle.path().to_path_buf(), InputGuard::Code(bundle))
        }
        Resolved::Site => match als_md::classify(path)? {
            Some(md_input) => {
                let rendered = als_md::render(md_input)?;
                (
                    rendered.path().to_path_buf(),
                    InputGuard::Rendered(rendered),
                )
            }
            None => (path.to_path_buf(), InputGuard::Plain),
        },
    };
    let input = classify(&pack_root)?;
    let packed = pack(input)?;

    let expires = match flags.expires.as_deref() {
        Some(s) => Some(parse_duration(s)?),
        None => None,
    };
    let (password_to_send, password_to_show) = resolve_password(flags.pass.as_deref());

    let client = Client::new(&cfg)?;

    // `site_id` priority on the wire (highest first):
    //   1. `--new`               → none (force fresh allocation)
    //   2. `als.toml.id`         → committed cross-machine pin
    //   3. local pin store id    → per-machine cache for this path
    //   4. none                  → unanchored; server falls back to
    //                              per-user case-insensitive name dedup.
    let site_id = if flags.new_site {
        None
    } else if let Some(id) = pin_ctx.config_id.clone() {
        Some(id)
    } else {
        pin_ctx.prior.as_ref().map(|p| p.id.clone())
    };

    // `project_name` policy:
    //   * Id-anchored deploy: only send a name when the user
    //     explicitly asked for a rename via `--name`. Without a flag,
    //     the server keeps its current name (the local pin's name
    //     may be stale after a cross-machine rename; the response
    //     surfaces the truth and we rewrite the pin from it).
    //   * Unanchored deploy (new site or `--new`): resolve a name
    //     from the existing chain — `--name` flag, `als.toml.name`,
    //     prior pin's recorded name (only reachable with `--new`,
    //     which forced `prior = None` upstream — so this collapses
    //     to "user-supplied or client-minted").
    let project_name = if site_id.is_some() {
        flags.name.clone()
    } else {
        Some(
            pin_ctx
                .effective_name
                .clone()
                .unwrap_or_else(default_project_name),
        )
    };

    let request = DeployRequest {
        site_id,
        project_name,
        expires,
        auth_password: password_to_send,
    };

    let bar = start_progress_bar(out.mode(), &packed.display_filename);
    let anchor_id = request.site_id.clone();
    let result = als_api::deploy(
        &client,
        packed.bytes,
        packed.display_filename.clone(),
        request,
    )
    .await;
    finish_progress_bar(bar);
    let response = match result {
        Ok(resp) => resp,
        // `ApiError::NotFound` from `/api/deploy` only happens on the
        // id-anchored branch: the wire `site_id` pointed at a site
        // the caller no longer owns (deleted, transferred, or
        // never existed). Surface the specific id plus a recovery
        // hint so the operator knows the pin/`als.toml` is stale.
        Err(ApiError::NotFound) => {
            let id = anchor_id.as_deref().unwrap_or("<unknown>");
            return Err(Error::NotFound {
                what: format!("site '{id}' on the server"),
                hint: Some(
                    "the id in als.toml or the local pin is stale; run with --new to allocate a fresh site"
                        .to_owned(),
                ),
            });
        }
        Err(other) => return Err(other.into()),
    };

    // Auto-write the server-assigned id back into `als.toml` so the
    // next deploy from any clone of this source tree resolves to the
    // same site without depending on the local pin cache. Only
    // meaningful for directory inputs — there is nowhere to put the
    // file for a single HTML / zip upload.
    let final_hash = if path.is_dir() && als_pack::AlsConfig::write_id(path, &response.id)? {
        // The write changed `als.toml` on disk; the digest folds in
        // that file, so re-fingerprint the source before the pin entry
        // is written. Otherwise the next deploy would compare the new
        // digest against the now-stale pre-write hash and skip the
        // no-op shortcut even when nothing else changed.
        als_pack::digest(path)?
    } else {
        content_hash
    };
    write_pin(&pin_ctx, &response, &final_hash)?;

    render(out, &response, password_to_show.as_deref())
}

/// `true` when the invocation carries no flags that would change what
/// the server stores. In that case a content-hash hit short-circuits.
/// Any of `--name`, `--new`, `--pass`, `--expires`, `--kind` flip this
/// to `false` and the deploy runs unconditionally.
fn is_metadata_only(flags: &DeployFlags) -> bool {
    !flags.new_site
        && flags.name.is_none()
        && flags.pass.is_none()
        && flags.expires.is_none()
        && flags.kind.is_none()
}

/// Identity inputs to one deploy call. The CLI's only job here is
/// "what `project_name` should we send to the server, and where do we
/// record the binding once the server has answered?" — version handling
/// is server-side.
struct PinContext {
    /// `path` canonicalised, used as the pin-store lookup key.
    canonical_path: PathBuf,
    /// Pin entry the canonical path was previously bound to, if any.
    /// Carried through so the no-op-skip branch can render without a
    /// second disk read.
    prior: Option<SiteEntry>,
    /// `als.toml.id` read from the input directory, if present and
    /// well-formed. Highest-priority anchor: when set, the wire
    /// `site_id` comes from here regardless of what the pin store
    /// holds for this path.
    config_id: Option<String>,
    /// The name the CLI passes as `project_name` on the wire. `None`
    /// means "no user-supplied or pin-derived name was found"; the
    /// caller then mints `<word><NNN>` locally before the upload.
    effective_name: Option<String>,
}

fn resolve_pin(path: &Path, flags: &DeployFlags) -> Result<PinContext, Error> {
    let canonical = path
        .canonicalize()
        .map_err(|e| Error::Other(format!("canonicalize '{}': {e}", path.display())))?;

    let prior = if flags.new_site {
        // `--new` forces a fresh site. Skip the pin lookup entirely so
        // the no-op skip branch never fires and the server always
        // allocates a new id.
        None
    } else {
        let entries = sites::load_all()?;
        sites::find_by_path(&entries, &canonical).cloned()
    };

    // Read `als.toml` once and pull both fields out. `--new` skips the
    // file entirely so the explicit "fresh site" flag never silently
    // anchors back onto the committed id.
    let cfg = if flags.new_site {
        None
    } else {
        load_config(path)?
    };
    let config_id = cfg.as_ref().and_then(|c| c.id.clone());
    let config_name = match cfg.as_ref().and_then(|c| c.name.clone()) {
        Some(name) => Some(
            crate::cli::validate_name(&name)
                .map_err(|e| Error::Config(format!("als.toml `name`: {e}")))?,
        ),
        None => None,
    };

    let effective_name = if flags.new_site {
        None
    } else if let Some(name) = flags.name.clone() {
        Some(name)
    } else if let Some(name) = config_name {
        // `als.toml` next to the input pins the project identity in
        // the repo itself, so a fresh clone on a different machine
        // resolves to the same site without needing the local
        // `~/.config/als/sites/` cache.
        Some(name)
    } else {
        prior.as_ref().map(|entry| entry.name.clone())
    };

    Ok(PinContext {
        canonical_path: canonical,
        prior,
        config_id,
        effective_name,
    })
}

/// Read `<path>/als.toml` if `<path>` is a directory. Wrapper around
/// [`als_pack::AlsConfig::load`] that returns `Ok(None)` for non-
/// directory inputs (single HTML / zip) which have no project root.
fn load_config(path: &Path) -> Result<Option<als_pack::AlsConfig>, Error> {
    if !path.is_dir() {
        return Ok(None);
    }
    als_pack::AlsConfig::load(path)
}

fn write_pin(ctx: &PinContext, resp: &DeployResponse, content_hash: &str) -> Result<(), Error> {
    // If the server returned a different name than the pin held (e.g.
    // user re-deployed with `--name OtherName`), drop the stale pin
    // file so we don't end up with two `*.toml` files pointing at the
    // same site id / canonical path.
    // Rename case: the server returned the request through under a
    // different name (typically because the user passed a fresh
    // `--name`). The pin filename embeds the lowercased name, so the
    // old `<id>_<old>.toml` would otherwise linger alongside the new
    // one. Remove by id — same site, just rewriting its file.
    if let Some(prior) = ctx.prior.as_ref()
        && !prior.name.eq_ignore_ascii_case(&resp.project_name)
    {
        sites::remove_by_id(&prior.id)?;
    }
    sites::upsert(&SiteEntry {
        name: resp.project_name.clone(),
        id: resp.id.clone(),
        path: ctx.canonical_path.to_string_lossy().into_owned(),
        url: Some(resp.url.clone()),
        last_published: current_iso(),
        last_content_hash: Some(content_hash.to_owned()),
    })
}

/// Encode the routing priority for `als <path>`:
///
/// 1. `--kind site` → forced [`Resolved::Site`].
/// 2. `--kind code` → forced [`Resolved::Code`]; errors when the
///    input is not code-shaped so the user gets a clear diagnostic.
/// 3. Markdown / mdbook input → [`Resolved::Site`] (existing als-md path).
/// 4. Pure-code input → [`Resolved::Code`].
/// 5. Otherwise → [`Resolved::Site`] (als-pack handles the rest).
fn resolve_kind<'a>(flags: &DeployFlags, path: &'a Path) -> Result<Resolved<'a>, Error> {
    match flags.kind {
        Some(KindFlag::Site) => Ok(Resolved::Site),
        Some(KindFlag::Code) => {
            // Forced override: bypass the html / md shape check. The user
            // explicitly wants the code-viewer rendering, even on inputs
            // that auto-detect would route to a static site.
            let input = als_code::force_input(path)?;
            Ok(Resolved::Code(input))
        }
        None => {
            // als_md::classify covers markdown / mdbook; if it claims the
            // path, never fall through to code (a directory full of
            // .md files is documentation, not code).
            if als_md::classify(path)?.is_some() {
                return Ok(Resolved::Site);
            }
            if let Some(input) = als_code::classify(path)? {
                return Ok(Resolved::Code(input));
            }
            Ok(Resolved::Site)
        }
    }
}

/// Translate the `--pass` value into `(send_to_server, show_to_user)`.
fn resolve_password(input: Option<&str>) -> (Option<String>, Option<String>) {
    match input {
        Some("auto") => {
            let generated = generate_memorable_password();
            (Some(generated.clone()), Some(generated))
        }
        Some(literal) => (Some(literal.to_owned()), None),
        None => (None, None),
    }
}

fn generate_memorable_password() -> String {
    let mut rng = rand::thread_rng();
    let mut groups: Vec<String> = Vec::with_capacity(AUTO_PASS_GROUPS);
    for _ in 0..AUTO_PASS_GROUPS {
        let group: String = (0..AUTO_PASS_GROUP_LEN)
            .map(|_| {
                let idx = rng.gen_range(0..AUTO_PASS_ALPHABET.len());
                char::from(AUTO_PASS_ALPHABET[idx])
            })
            .collect();
        groups.push(group);
    }
    groups.join("-")
}

/// Short, pronounceable, lowercase words used to build a memorable
/// default project name. Mixed animals / colors / nature nouns; each
/// is 3-5 letters so the resulting `<word><NNN>` stays under ~9 chars.
///
/// 64 words × 1000 digit combos = 64 000 default-name shapes per user.
/// The server is the sole arbiter of uniqueness — if it ever rejects
/// the candidate, the CLI surfaces the error and the user can retry
/// with their own `--name`.
const DEFAULT_NAME_WORDS: &[&str] = &[
    "fox", "cat", "dog", "owl", "bee", "ant", "bat", "elk", "hen", "pig", "cow", "jay", "wolf",
    "lion", "hawk", "lynx", "mole", "swan", "crow", "dove", "duck", "goat", "panda", "robin",
    "tiger", "eagle", "koala", "llama", "moose", "snake", "whale", "raven", "red", "blue", "jade",
    "mint", "gold", "navy", "ruby", "teal", "sage", "plum", "coral", "amber", "leaf", "moon",
    "star", "wave", "fire", "rain", "snow", "sun", "peak", "cloud", "lake", "hill", "frost",
    "dawn", "dusk", "ember", "stone", "reef", "fern", "oak",
];
/// Three-digit numeric tail (`000`..`999`).
const DEFAULT_NAME_DIGITS: usize = 3;

/// Build the default project name used when the deploy carries no
/// `--name`, no `als.toml` name, and no prior pin: `<word><NNN>` —
/// concatenated, no separator (e.g. `fox042`, `mint007`, `panda500`).
/// Server-only uniqueness — no `account/me` round trip, no
/// client-side dedup. If the candidate happens to collide for this
/// user the server returns the matching site under the same id and
/// the deploy pushes a version (the standard re-deploy behaviour).
fn default_project_name() -> String {
    let mut rng = rand::thread_rng();
    let word = DEFAULT_NAME_WORDS[rng.gen_range(0..DEFAULT_NAME_WORDS.len())];
    let mut digits = String::with_capacity(DEFAULT_NAME_DIGITS);
    for _ in 0..DEFAULT_NAME_DIGITS {
        digits.push(char::from(b'0' + rng.gen_range(0..10_u8)));
    }
    format!("{word}{digits}")
}

fn start_progress_bar(mode: OutputMode, filename: &str) -> Option<ProgressBar> {
    if mode != OutputMode::Human || !std::io::stderr().is_terminal() {
        return None;
    }
    let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
    if let Ok(style) = ProgressStyle::with_template("{spinner} uploading {msg}") {
        bar.set_style(style);
    }
    bar.enable_steady_tick(Duration::from_millis(PROGRESS_TICK_MS));
    bar.set_message(filename.to_owned());
    Some(bar)
}

fn finish_progress_bar(bar: Option<ProgressBar>) {
    if let Some(b) = bar {
        b.finish_and_clear();
    }
}

/// Render the "no changes since last deploy" outcome triggered by a
/// content-hash hit. The prior pin entry is the source of truth here
/// — there is no fresh server response to surface.
fn render_no_change(out: &mut Output, prior: &SiteEntry) -> Result<(), Error> {
    let url = prior.url.as_deref().unwrap_or("");
    match out.mode() {
        OutputMode::Quiet => {
            // Quiet mode is shell-pipeable; print the URL just like a
            // successful deploy so scripts don't need to special-case
            // the skip path.
            out.quiet_line(url);
            Ok(())
        }
        OutputMode::Json => out.json(&NoChangeJson {
            id: &prior.id,
            url,
            name: &prior.name,
            skipped: true,
        }),
        OutputMode::Human => {
            let id = prior.id.clone();
            let name = prior.name.clone();
            let url = url.to_owned();
            out.human(move |w| {
                let _ = writeln!(w, "\x1b[33m·\x1b[0m No changes since last deploy: {url}");
                let _ = writeln!(w, "  Id:      {id}");
                let _ = writeln!(w, "  Name:    {name}");
                let _ = writeln!(w, "  (run with --new for a fresh site, or --pass/--expires/--name to update metadata)");
            });
            Ok(())
        }
    }
}

#[derive(Serialize)]
struct NoChangeJson<'a> {
    id: &'a str,
    url: &'a str,
    name: &'a str,
    skipped: bool,
}

fn render(out: &mut Output, resp: &DeployResponse, password: Option<&str>) -> Result<(), Error> {
    match out.mode() {
        OutputMode::Quiet => {
            out.quiet_line(&resp.url);
            Ok(())
        }
        OutputMode::Json => out.json(&DeployJson {
            response: resp,
            password,
        }),
        OutputMode::Human => {
            let now_iso = current_iso();
            out.human(|w| write_human(w, resp, password, &now_iso));
            Ok(())
        }
    }
}

fn write_human(w: &mut dyn Write, resp: &DeployResponse, password: Option<&str>, now_iso: &str) {
    let _ = writeln!(w, "\x1b[32m✓\x1b[0m Deployed: {}", resp.url);
    // Two distinct identifiers, surfaced separately so users can tell
    // them apart at a glance:
    //   * Id   — 10-char base32 nanoid; the URL subdomain; immutable.
    //   * Name — human-readable label set by `--name`; mutable via
    //            `als site <id|name> --name <new>`.
    let _ = writeln!(w, "  Id:      {}", resp.id);
    let _ = writeln!(w, "  Name:    {}", resp.project_name);
    // Server-assigned version id. Display-only — the CLI cannot choose
    // a version on upload. Use `als site <id> --version <v>` to roll
    // back / forward by id.
    let _ = writeln!(w, "  Version: {}", resp.version);
    if let Some(p) = password {
        let _ = writeln!(w, "  Password: {p}");
    }
    let _ = writeln!(
        w,
        "  Expires: {}",
        format_expires(resp.expires_at.as_deref(), now_iso)
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn resolve_password_none_yields_none() {
        let (send, show) = resolve_password(None);
        assert!(send.is_none());
        assert!(show.is_none());
    }

    #[test]
    fn resolve_password_literal_sends_but_does_not_show() {
        let (send, show) = resolve_password(Some("s3cret"));
        assert_eq!(send.as_deref(), Some("s3cret"));
        assert!(show.is_none());
    }

    #[test]
    fn resolve_password_auto_generates_and_shows_same_value() {
        let (send, show) = resolve_password(Some("auto"));
        let send = send.expect("send");
        let show = show.expect("show");
        assert_eq!(send, show, "generated password must match what we display");
        assert_eq!(
            send.len(),
            AUTO_PASS_GROUPS * AUTO_PASS_GROUP_LEN + AUTO_PASS_GROUPS - 1
        );
    }

    #[test]
    fn generated_password_format_is_grouped_with_hyphens() {
        let pw = generate_memorable_password();
        let groups: Vec<&str> = pw.split('-').collect();
        assert_eq!(groups.len(), AUTO_PASS_GROUPS);
        for g in &groups {
            assert_eq!(g.len(), AUTO_PASS_GROUP_LEN);
            assert!(
                g.bytes().all(|b| AUTO_PASS_ALPHABET.contains(&b)),
                "unexpected char in {g}"
            );
        }
    }

    #[test]
    fn default_project_name_is_word_plus_digits() {
        for _ in 0..32 {
            let name = default_project_name();
            assert!(name.len() > DEFAULT_NAME_DIGITS, "{name} is too short");
            // Trailing `DEFAULT_NAME_DIGITS` digits.
            let tail = &name[name.len() - DEFAULT_NAME_DIGITS..];
            assert!(
                tail.chars().all(|c| c.is_ascii_digit()),
                "{name} should end in {DEFAULT_NAME_DIGITS} digits"
            );
            // Leading word is one of the curated entries.
            let word = &name[..name.len() - DEFAULT_NAME_DIGITS];
            assert!(
                DEFAULT_NAME_WORDS.contains(&word),
                "{word} should be one of the curated words"
            );
            // The whole thing must pass the `--name` validator.
            crate::cli::validate_name(&name).expect("default name passes validator");
        }
    }

    #[test]
    fn default_word_list_is_lowercase_alphabetic() {
        for word in DEFAULT_NAME_WORDS {
            assert!(
                word.chars().all(|c| c.is_ascii_lowercase()),
                "{word} must be lowercase ASCII letters"
            );
            assert!(!word.is_empty(), "word entries cannot be empty");
        }
    }
}
