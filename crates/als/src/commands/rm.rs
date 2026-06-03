//! `als rm <id|name>` — remove one site, or every expired site with
//! `--all-expired`.

use als_api::{ApiError, Client, ListSitesQuery, Site, SiteState, delete, get, list};
use als_core::sites;
use als_core::{Error, Output, OutputMode};
use serde_json::{Value, json};

use crate::cli::RmArgs;
use crate::commands::list::current_iso;
use crate::commands::site_ident;
use crate::prompt::confirm;

const HUMAN_REMOVED_SINGLE: &str = "✓ Removed (will be permanently deleted in 24h cooldown).";

pub(crate) async fn run(client: &Client, out: &mut Output, args: RmArgs) -> Result<(), Error> {
    if args.all_expired {
        run_all_expired(client, out, args.yes).await
    } else {
        let arg = args
            .id_or_name
            .ok_or_else(|| Error::Other("missing site id or name".into()))?;
        let id = site_ident::resolve(client, &arg).await?;
        run_single(client, out, &id, args.yes).await
    }
}

async fn run_single(client: &Client, out: &mut Output, id: &str, yes: bool) -> Result<(), Error> {
    let site = get(client, id).await.map_err(|e| map_err(e, id))?;
    if !yes && !confirm_single(&site)? {
        return Ok(());
    }
    delete(client, id).await.map_err(|e| map_err(e, id))?;
    drop_local_pins(&[id.to_owned()]);
    render_single(out, id)
}

async fn run_all_expired(client: &Client, out: &mut Output, yes: bool) -> Result<(), Error> {
    let query = ListSitesQuery {
        limit: Some(200),
        ..ListSitesQuery::default()
    };
    let resp = list(client, &query).await.map_err(Error::from)?;
    let now_iso = current_iso();
    let ids: Vec<String> = resp
        .data
        .into_iter()
        .filter(|s| SiteState::derive(s, &now_iso) == SiteState::Expired)
        .map(|s| s.id)
        .collect();

    if ids.is_empty() {
        return render_batch(out, &[]);
    }
    if !yes && !confirm_batch(ids.len())? {
        return Ok(());
    }

    let mut removed: Vec<String> = Vec::with_capacity(ids.len());
    for id in &ids {
        delete(client, id).await.map_err(|e| map_err(e, id))?;
        removed.push(id.clone());
    }
    drop_local_pins(&removed);
    render_batch(out, &removed)
}

/// Best-effort cleanup of `~/.config/als/sites/*.toml` entries pointing
/// at sites we just deleted. Errors are silently ignored — the user
/// already accepted the server-side removal, so a dangling local pin
/// shouldn't surface a hard error.
fn drop_local_pins(site_ids: &[String]) {
    for id in site_ids {
        let _ = sites::remove_by_key(id);
    }
}

fn map_err(e: ApiError, id: &str) -> Error {
    match e {
        ApiError::NotFound => Error::NotFound {
            what: format!("site '{id}'"),
            hint: None,
        },
        other => other.into(),
    }
}

fn confirm_single(site: &Site) -> Result<bool, Error> {
    confirm(&format!(
        "Confirm delete '{}' (name: {})?",
        site.full_domain, site.project_name,
    ))
}

fn confirm_batch(n: usize) -> Result<bool, Error> {
    confirm(&format!("Remove {n} expired site(s)?"))
}

fn render_single(out: &mut Output, id: &str) -> Result<(), Error> {
    match out.mode() {
        OutputMode::Json => out.json(&single_payload(id)),
        OutputMode::Human => {
            out.quiet_line(HUMAN_REMOVED_SINGLE);
            Ok(())
        }
        OutputMode::Quiet => Ok(()),
    }
}

fn render_batch(out: &mut Output, ids: &[String]) -> Result<(), Error> {
    match out.mode() {
        OutputMode::Json => out.json(&batch_payload(ids)),
        OutputMode::Human => {
            out.quiet_line(&batch_human(ids.len()));
            Ok(())
        }
        OutputMode::Quiet => Ok(()),
    }
}

fn single_payload(id: &str) -> Value {
    json!({ "removed": [id] })
}

fn batch_payload(ids: &[String]) -> Value {
    json!({ "removed": ids })
}

fn batch_human(n: usize) -> String {
    if n == 0 {
        "✓ No expired sites to remove.".to_owned()
    } else {
        format!("✓ Removed {n} expired site(s).")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_err_not_found_carries_site_id() {
        let err = map_err(ApiError::NotFound, "k7x2qm4j6p");
        match err {
            Error::NotFound { what, hint } => {
                assert_eq!(what, "site 'k7x2qm4j6p'");
                assert!(hint.is_none());
            }
            other => unreachable!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn map_err_other_passes_through_default_conversion() {
        let err = map_err(ApiError::Unauthorized, "k7x2qm4j6p");
        assert!(matches!(err, Error::Auth));
    }

    #[test]
    fn single_payload_wraps_id_in_removed_array() {
        assert_eq!(
            single_payload("k7x2qm4j6p"),
            json!({ "removed": ["k7x2qm4j6p"] }),
        );
    }

    #[test]
    fn batch_payload_carries_every_id_in_order() {
        let ids = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(batch_payload(&ids), json!({ "removed": ["a", "b", "c"] }),);
    }

    #[test]
    fn batch_human_pluralizes_with_count() {
        assert_eq!(batch_human(1), "✓ Removed 1 expired site(s).");
        assert_eq!(batch_human(7), "✓ Removed 7 expired site(s).");
    }

    #[test]
    fn batch_human_zero_uses_no_op_message() {
        assert_eq!(batch_human(0), "✓ No expired sites to remove.");
    }

    #[test]
    fn human_removed_single_matches_spec_wording() {
        assert_eq!(
            HUMAN_REMOVED_SINGLE,
            "✓ Removed (will be permanently deleted in 24h cooldown).",
        );
    }
}
