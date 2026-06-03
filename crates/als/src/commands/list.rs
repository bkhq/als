//! `als list` — render the caller's sites in a table.
//!
//! Wire transport is [`als_api::endpoints::sites::list`]. The server lacks a
//! `state` query parameter, so the CLI derives state client-side from
//! `expiresAt` / `releasedAt`. `--auth` filtering is also client-side.

use std::fmt::Write;
use std::time::SystemTime;

use als_api::endpoints::sites;
use als_api::{Client, ListSitesQuery, ListSitesResponse, Site, SiteState};
use als_core::{Error, Output, OutputMode};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::cli::{AuthArg, ListArgs, StateArg};

pub(crate) async fn run(client: &Client, out: &mut Output, args: ListArgs) -> Result<(), Error> {
    let query = ListSitesQuery {
        limit: Some(args.limit),
        page: None,
        all: false,
    };
    let mut resp: ListSitesResponse = sites::list(client, &query).await?;
    let now_iso = current_iso();
    filter_by_state(&mut resp.data, args.state, &now_iso);
    filter_by_auth(&mut resp.data, args.auth);

    match out.mode() {
        OutputMode::Json => out.json(&JsonView {
            data: &resp.data,
            total: resp.meta.total,
        })?,
        OutputMode::Human | OutputMode::Quiet => render_text(out, &resp, &now_iso),
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct JsonView<'a> {
    data: &'a [Site],
    total: u32,
}

fn filter_by_state(items: &mut Vec<Site>, filter: StateArg, now_iso: &str) {
    if matches!(filter, StateArg::All) {
        return;
    }
    let want = match filter {
        StateArg::Active => SiteState::Active,
        StateArg::Expired => SiteState::Expired,
        StateArg::Released => SiteState::Released,
        StateArg::All => return,
    };
    items.retain(|s| SiteState::derive(s, now_iso) == want);
}

fn filter_by_auth(items: &mut Vec<Site>, filter: Option<AuthArg>) {
    let Some(f) = filter else { return };
    let want_auth = matches!(f, AuthArg::Pass);
    items.retain(|s| s.auth_required == want_auth);
}

struct Row {
    id: String,
    project: String,
    url: String,
    expires: String,
    auth: &'static str,
}

const HEADERS: [&str; 5] = ["ID", "NAME", "URL", "EXPIRES", "AUTH"];

fn render_text(out: &mut Output, resp: &ListSitesResponse, now_iso: &str) {
    let rows: Vec<Row> = resp.data.iter().map(|s| site_to_row(s, now_iso)).collect();
    if !rows.is_empty() {
        out.quiet_line(&render_table(&rows));
    }
    out.quiet_line(&format!(
        "{} site{} (total {})",
        resp.data.len(),
        if resp.data.len() == 1 { "" } else { "s" },
        resp.meta.total
    ));
}

fn site_to_row(site: &Site, now_iso: &str) -> Row {
    Row {
        id: site.id.clone(),
        project: site.project_name.clone(),
        url: site.full_domain.clone(),
        expires: format_expires(site.expires_at.as_deref(), now_iso),
        auth: if site.auth_required { "pass" } else { "none" },
    }
}

/// Render the rows as a left-aligned, space-padded plain-text table.
/// One header row, no borders, two-space gap between columns —
/// hand-rolled to avoid pulling in a table-rendering crate for the
/// one consumer that needs one.
fn render_table(rows: &[Row]) -> String {
    let mut widths = HEADERS.map(str::len);
    for r in rows {
        widths[0] = widths[0].max(r.id.len());
        widths[1] = widths[1].max(r.project.len());
        widths[2] = widths[2].max(r.url.len());
        widths[3] = widths[3].max(r.expires.len());
        widths[4] = widths[4].max(r.auth.len());
    }

    let mut out = String::new();
    write_row(&mut out, &widths, &HEADERS);
    for r in rows {
        write_row(
            &mut out,
            &widths,
            &[&r.id, &r.project, &r.url, &r.expires, r.auth],
        );
    }
    // Drop the trailing newline; `Output::quiet_line` appends its own.
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

fn write_row(out: &mut String, widths: &[usize; 5], cells: &[&str; 5]) {
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push_str("  ");
        }
        let w = widths[i];
        if i + 1 == cells.len() {
            // Last column: skip trailing padding for a tighter line.
            out.push_str(cell);
        } else {
            let _ = write!(out, "{cell:<w$}");
        }
    }
    out.push('\n');
}

pub(crate) fn format_expires(expires_iso: Option<&str>, now_iso: &str) -> String {
    let Some(iso) = expires_iso else {
        return "never".to_owned();
    };
    let Some(at) = parse_iso_seconds(iso) else {
        return iso.to_owned();
    };
    let Some(now) = parse_iso_seconds(now_iso) else {
        return iso.to_owned();
    };
    let delta = at - now;
    if delta <= 0 {
        return "expired".to_owned();
    }
    let days = delta / 86_400;
    let hours = (delta % 86_400) / 3_600;
    let minutes = (delta % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

pub(crate) fn parse_iso_seconds(iso: &str) -> Option<i64> {
    OffsetDateTime::parse(iso, &Rfc3339)
        .ok()
        .map(OffsetDateTime::unix_timestamp)
}

pub(crate) fn current_iso() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let secs = i64::try_from(now).unwrap_or(i64::MAX);
    OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .and_then(|d| d.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn site(id: &str, auth_required: bool, expires: Option<&str>, released: Option<&str>) -> Site {
        Site {
            id: id.to_owned(),
            full_domain: format!("{id}.a.ls"),
            project_name: format!("project-{id}"),
            description: None,
            auth_required,
            current_version: "v1".to_owned(),
            created_at: "2026-05-01T00:00:00Z".to_owned(),
            updated_at: "2026-05-01T00:00:00Z".to_owned(),
            expires_at: expires.map(str::to_owned),
            released_at: released.map(str::to_owned),
        }
    }

    #[test]
    fn filter_by_auth_pass_drops_public() {
        let mut items = vec![
            site("a", false, None, None),
            site("b", true, None, None),
            site("c", true, None, None),
        ];
        filter_by_auth(&mut items, Some(AuthArg::Pass));
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|s| s.auth_required));
    }

    #[test]
    fn filter_by_state_active_keeps_only_active() {
        let mut items = vec![
            site("a", false, Some("2099-01-01T00:00:00Z"), None),
            site("b", false, Some("2020-01-01T00:00:00Z"), None),
            site("c", false, None, Some("2025-01-01T00:00:00Z")),
        ];
        filter_by_state(&mut items, StateArg::Active, "2026-05-10T00:00:00Z");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "a");
    }

    #[test]
    fn filter_by_state_all_passthrough() {
        let mut items = vec![
            site("a", false, Some("2099-01-01T00:00:00Z"), None),
            site("b", false, Some("2020-01-01T00:00:00Z"), None),
        ];
        filter_by_state(&mut items, StateArg::All, "2026-05-10T00:00:00Z");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn format_expires_never() {
        assert_eq!(format_expires(None, "2026-05-10T00:00:00Z"), "never");
    }

    #[test]
    fn format_expires_past_is_expired() {
        assert_eq!(
            format_expires(Some("2020-01-01T00:00:00Z"), "2026-05-10T00:00:00Z"),
            "expired"
        );
    }

    #[test]
    fn format_expires_days() {
        // exactly 6d 0h from 2026-05-10
        let v = format_expires(Some("2026-05-16T00:00:00Z"), "2026-05-10T00:00:00Z");
        assert_eq!(v, "6d 0h");
    }

    #[test]
    fn format_expires_hours_minutes() {
        let v = format_expires(Some("2026-05-10T05:30:00Z"), "2026-05-10T00:00:00Z");
        assert_eq!(v, "5h 30m");
    }

    #[test]
    fn parse_iso_seconds_basic() {
        let s = parse_iso_seconds("2026-05-10T00:00:00Z").unwrap();
        // Within rounding distance of the expected day.
        assert!(s > 1_700_000_000);
    }
}
