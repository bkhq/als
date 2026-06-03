//! Closed error enum for als-core and downstream crates, plus the
//! `ExitCode` trait that maps each variant to a process exit code per
//! `docs/cli-spec.md`. Code `2` is reserved for clap parse errors and is
//! handled in the binary crate, not here.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("config: {0}")]
    Config(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("auth required or token invalid")]
    Auth,
    /// Server returned 404 for a referenced resource. `what` identifies
    /// the missing resource (e.g. `"site 'k7x2qm4j6p'"`) so the stderr
    /// message can read `not_found: site 'k7x2qm4j6p' not found`. The
    /// optional `hint` is rendered as the `Hint:` line under it; callers
    /// fill it in when they know what to suggest (stale id pin, version
    /// not on this site, …).
    #[error("not found: {what}")]
    NotFound { what: String, hint: Option<String> },
    #[error("network: {0}")]
    Network(String),
    #[error("server error: {0}")]
    Server(String),
    #[error("quota exceeded")]
    Quota,
    #[error("rate limited; retry after {0}s")]
    RateLimited(u64),
    #[error("{0}")]
    Other(String),
}

pub trait ExitCode {
    fn exit_code(&self) -> i32;
}

impl ExitCode for Error {
    fn exit_code(&self) -> i32 {
        match self {
            Error::Other(_) | Error::Config(_) | Error::Io(_) | Error::NotFound { .. } => 1,
            Error::Auth => 3,
            Error::Network(_) => 4,
            Error::Server(_) => 5,
            Error::Quota => 6,
            Error::RateLimited(_) => 7,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_maps_to_1() {
        assert_eq!(Error::Config("bad".into()).exit_code(), 1);
    }

    #[test]
    fn io_maps_to_1() {
        let err = Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "x"));
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn other_maps_to_1() {
        assert_eq!(Error::Other("boom".into()).exit_code(), 1);
    }

    #[test]
    fn auth_maps_to_3() {
        assert_eq!(Error::Auth.exit_code(), 3);
    }

    #[test]
    fn not_found_maps_to_1() {
        assert_eq!(
            Error::NotFound {
                what: "site 'x'".into(),
                hint: None,
            }
            .exit_code(),
            1
        );
    }

    #[test]
    fn network_maps_to_4() {
        assert_eq!(Error::Network("dns".into()).exit_code(), 4);
    }

    #[test]
    fn server_maps_to_5() {
        assert_eq!(Error::Server("500".into()).exit_code(), 5);
    }

    #[test]
    fn quota_maps_to_6() {
        assert_eq!(Error::Quota.exit_code(), 6);
    }

    #[test]
    fn rate_limited_maps_to_7() {
        assert_eq!(Error::RateLimited(30).exit_code(), 7);
    }

    #[test]
    fn from_io_error_works() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err: Error = io.into();
        assert!(matches!(err, Error::Io(_)));
        assert_eq!(err.exit_code(), 1);
    }
}
