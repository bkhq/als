//! Parse `Expires` strings accepted by `als <path> --expires` and
//! `als site <id> --expires <duration>` (see `docs/cli-spec.md`).
//!
//! Accepted forms: `<int>{s|m|h|d|y}` (e.g. `5m`, `168h`, `7d`, `1y`), the
//! literal `never` (case-insensitive), and RFC 3339 / ISO 8601 timestamps.
//! `Display` round-trips to a canonical spec string: `never`; `<n>h` when
//! the duration is hour-aligned, else `<n>m`, else `<n>s`; RFC 3339 for the
//! absolute form. Years are approximated as 365 days for the `Duration`
//! variant — use the absolute form when precision matters.

use std::fmt;
use std::time::Duration;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::Error;

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3_600;
const SECONDS_PER_DAY: u64 = 86_400;
const SECONDS_PER_YEAR: u64 = 365 * SECONDS_PER_DAY;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expires {
    Duration(Duration),
    Never,
    AbsoluteUtc(OffsetDateTime),
}

pub fn parse(input: &str) -> Result<Expires, Error> {
    if input.is_empty() {
        return Err(invalid(input));
    }
    if input.eq_ignore_ascii_case("never") {
        return Ok(Expires::Never);
    }
    if let Some(duration) = parse_shorthand(input) {
        return Ok(Expires::Duration(duration));
    }
    OffsetDateTime::parse(input, &Rfc3339)
        .map(Expires::AbsoluteUtc)
        .map_err(|_| invalid(input))
}

fn parse_shorthand(input: &str) -> Option<Duration> {
    let last = input.as_bytes().last().copied()?;
    let multiplier: u64 = match last {
        b's' => 1,
        b'm' => SECONDS_PER_MINUTE,
        b'h' => SECONDS_PER_HOUR,
        b'd' => SECONDS_PER_DAY,
        b'y' => SECONDS_PER_YEAR,
        _ => return None,
    };
    let value_part = &input[..input.len() - 1];
    if value_part.is_empty() {
        return None;
    }
    if !value_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: u64 = value_part.parse().ok()?;
    let secs = value.checked_mul(multiplier)?;
    Some(Duration::from_secs(secs))
}

fn invalid(input: &str) -> Error {
    Error::Other(format!("invalid duration: {input:?}"))
}

impl fmt::Display for Expires {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expires::Never => f.write_str("never"),
            Expires::Duration(d) => {
                let secs = d.as_secs();
                if secs % SECONDS_PER_HOUR == 0 {
                    write!(f, "{}h", secs / SECONDS_PER_HOUR)
                } else if secs % SECONDS_PER_MINUTE == 0 {
                    write!(f, "{}m", secs / SECONDS_PER_MINUTE)
                } else {
                    write!(f, "{secs}s")
                }
            }
            Expires::AbsoluteUtc(dt) => {
                let formatted = dt.format(&Rfc3339).map_err(|_| fmt::Error)?;
                f.write_str(&formatted)
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::duration_suboptimal_units
)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn duration(input: &str) -> Duration {
        match parse(input) {
            Ok(Expires::Duration(d)) => d,
            other => panic!("expected Duration variant for {input:?}, got {other:?}"),
        }
    }

    #[test]
    fn shorthand_5m() {
        assert_eq!(duration("5m"), Duration::from_secs(5 * 60));
    }

    #[test]
    fn shorthand_1h() {
        assert_eq!(duration("1h"), Duration::from_secs(3_600));
    }

    #[test]
    fn shorthand_24h() {
        assert_eq!(duration("24h"), Duration::from_secs(24 * 3_600));
    }

    #[test]
    fn shorthand_1d() {
        assert_eq!(duration("1d"), Duration::from_secs(86_400));
    }

    #[test]
    fn shorthand_7d() {
        assert_eq!(duration("7d"), Duration::from_secs(7 * 86_400));
    }

    #[test]
    fn shorthand_30d() {
        assert_eq!(duration("30d"), Duration::from_secs(30 * 86_400));
    }

    #[test]
    fn shorthand_1y_is_365_days() {
        assert_eq!(duration("1y"), Duration::from_secs(365 * 86_400));
    }

    #[test]
    fn generic_seconds_unit() {
        assert_eq!(duration("90s"), Duration::from_secs(90));
    }

    #[test]
    fn generic_large_hours() {
        assert_eq!(duration("168h"), Duration::from_secs(168 * 3_600));
    }

    #[test]
    fn never_lowercase() {
        assert_eq!(parse("never").unwrap(), Expires::Never);
    }

    #[test]
    fn never_uppercase_is_case_insensitive() {
        assert_eq!(parse("NEVER").unwrap(), Expires::Never);
        assert_eq!(parse("Never").unwrap(), Expires::Never);
    }

    #[test]
    fn rfc3339_zulu() {
        let parsed = parse("2026-12-31T00:00:00Z").unwrap();
        assert_eq!(
            parsed,
            Expires::AbsoluteUtc(datetime!(2026-12-31 00:00:00 UTC)),
        );
    }

    #[test]
    fn rfc3339_with_offset() {
        let parsed = parse("2026-12-31T08:00:00+08:00").unwrap();
        assert_eq!(
            parsed,
            Expires::AbsoluteUtc(datetime!(2026-12-31 08:00:00 +08:00)),
        );
    }

    #[test]
    fn rejects_empty() {
        assert!(parse("").is_err());
    }

    #[test]
    fn rejects_unknown_word() {
        assert!(parse("abc").is_err());
    }

    #[test]
    fn rejects_negative_value() {
        assert!(parse("-5h").is_err());
    }

    #[test]
    fn rejects_positive_sign() {
        assert!(parse("+5h").is_err());
    }

    #[test]
    fn rejects_compound() {
        assert!(parse("1m30s").is_err());
    }

    #[test]
    fn rejects_bare_unit() {
        assert!(parse("h").is_err());
    }

    #[test]
    fn rejects_bare_integer() {
        assert!(parse("5").is_err());
    }

    #[test]
    fn rejects_unknown_unit() {
        assert!(parse("5w").is_err());
    }

    #[test]
    fn rejects_unit_uppercase() {
        assert!(parse("5H").is_err());
    }

    #[test]
    fn rejects_fractional() {
        assert!(parse("1.5h").is_err());
    }

    #[test]
    fn rejects_overflow() {
        assert!(parse("99999999999999y").is_err());
    }

    #[test]
    fn display_never() {
        assert_eq!(Expires::Never.to_string(), "never");
    }

    #[test]
    fn display_hours_for_hour_aligned() {
        let d = Expires::Duration(Duration::from_secs(168 * 3_600));
        assert_eq!(d.to_string(), "168h");
    }

    #[test]
    fn display_minutes_when_not_hour_aligned() {
        let d = Expires::Duration(Duration::from_secs(5 * 60));
        assert_eq!(d.to_string(), "5m");
    }

    #[test]
    fn display_seconds_when_not_minute_aligned() {
        let d = Expires::Duration(Duration::from_secs(90));
        assert_eq!(d.to_string(), "90s");
    }

    #[test]
    fn display_absolute_rfc3339() {
        let dt = datetime!(2026-12-31 00:00:00 UTC);
        assert_eq!(Expires::AbsoluteUtc(dt).to_string(), "2026-12-31T00:00:00Z");
    }

    #[test]
    fn roundtrip_5m() {
        let parsed = parse("5m").unwrap();
        assert_eq!(parsed.to_string(), "5m");
    }

    #[test]
    fn roundtrip_7d_canonicalizes_to_168h() {
        let parsed = parse("7d").unwrap();
        assert_eq!(parsed.to_string(), "168h");
    }

    #[test]
    fn roundtrip_30d_canonicalizes_to_720h() {
        let parsed = parse("30d").unwrap();
        assert_eq!(parsed.to_string(), "720h");
    }

    #[test]
    fn roundtrip_1y_canonicalizes_to_8760h() {
        let parsed = parse("1y").unwrap();
        assert_eq!(parsed.to_string(), "8760h");
    }

    #[test]
    fn roundtrip_never() {
        let parsed = parse("Never").unwrap();
        assert_eq!(parsed.to_string(), "never");
    }

    #[test]
    fn roundtrip_rfc3339() {
        let parsed = parse("2026-12-31T00:00:00Z").unwrap();
        assert_eq!(parsed.to_string(), "2026-12-31T00:00:00Z");
    }
}
