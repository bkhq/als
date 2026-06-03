//! API error taxonomy.
//!
//! `ApiError` is the closed enum that every endpoint surfaces. It mirrors
//! the wire-level error codes used by the live als server plus
//! transport-layer failures (`Network`, `Decode`). The conversion into
//! `als_core::Error` keeps the binary crate's exit-code mapping a pure
//! function of `als_core::Error`; see `docs/cli-spec.md` for the
//! authoritative error-code -> exit-code matrix.

use crate::envelope::ApiErrorPayload;

/// Errors surfaced by every `als-api` endpoint.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Transport-level failure (DNS, TLS, timeout, connection reset, …).
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    /// Response body could not be parsed as the documented envelope.
    #[error("decode: {0}")]
    Decode(String),
    /// 401 `UNAUTHORIZED` — token missing or invalid.
    #[error("auth required")]
    Unauthorized,
    /// 403 `FORBIDDEN` / `ACCOUNT_DISABLED` — caller lacks permission.
    #[error("forbidden")]
    Forbidden,
    /// 404 `NOT_FOUND` — referenced resource is unknown.
    #[error("not found")]
    NotFound,
    /// 400 / 422 `VALIDATION_ERROR` / `INVALID_REQUEST` — request payload
    /// failed validation.
    #[error("invalid request: {0}")]
    BadRequest(String),
    /// 410 `EXPIRED` / `PAIRING_EXPIRED` — site or pairing session expired.
    #[error("expired")]
    Expired,
    /// 413 `ARCHIVE_TOO_LARGE`.
    #[error("archive too large")]
    ArchiveTooLarge,
    /// 400 `ARCHIVE_PATH_TRAVERSAL` — zip contained `..` / absolute /
    /// symlink entry.
    #[error("path traversal")]
    PathTraversal,
    /// 402 `QUOTA_EXCEEDED`.
    #[error("quota exceeded")]
    QuotaExceeded,
    /// 429 `RATE_LIMITED`. The inner value is the `Retry-After` header in
    /// seconds (or `0` if the server omitted it).
    #[error("rate limited; retry after {0}s")]
    RateLimited(u64),
    /// 500 `INTERNAL_ERROR`.
    #[error("server error")]
    Internal,
    /// Any error code not explicitly modelled above. Preserves the
    /// server-supplied `code` / `message` so logs and the user-facing CLI
    /// can still display something useful.
    #[error("server: {code} - {message}")]
    Other { code: String, message: String },
}

impl From<ApiError> for als_core::Error {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::Unauthorized => als_core::Error::Auth,
            ApiError::NotFound => als_core::Error::NotFound {
                what: "resource".into(),
                hint: None,
            },
            ApiError::Network(ref inner) => als_core::Error::Network(inner.to_string()),
            ApiError::Internal | ApiError::Other { .. } => als_core::Error::Server(e.to_string()),
            ApiError::QuotaExceeded => als_core::Error::Quota,
            ApiError::RateLimited(s) => als_core::Error::RateLimited(s),
            other => als_core::Error::Other(other.to_string()),
        }
    }
}

/// Map a parsed `{ code, message }` envelope plus its HTTP status into the
/// matching `ApiError` variant.
///
/// `RateLimited` is constructed with `0` here; the client patches in the
/// real `Retry-After` value at the call site because that header lives
/// outside the JSON body.
pub(crate) fn from_envelope(payload: ApiErrorPayload, _status: reqwest::StatusCode) -> ApiError {
    match payload.code.as_str() {
        "UNAUTHORIZED" | "AUTH_REQUIRED" | "PASSWORD_WRONG" | "PASSWORD_REQUIRED"
        | "OIDC_FAILED" => ApiError::Unauthorized,
        "FORBIDDEN" | "ACCOUNT_DISABLED" => ApiError::Forbidden,
        "NOT_FOUND" => ApiError::NotFound,
        "VALIDATION_ERROR" | "INVALID_REQUEST" | "INVALID_MULTIPART" => {
            ApiError::BadRequest(payload.message)
        }
        "ARCHIVE_PATH_TRAVERSAL"
        | "ARCHIVE_PATH_INVALID"
        | "ARCHIVE_SYMLINK"
        | "ARCHIVE_ENCRYPTED"
        | "ARCHIVE_COMPRESSION_UNSUPPORTED"
        | "ARCHIVE_ZIP64_UNSUPPORTED"
        | "ARCHIVE_PATH_TOO_DEEP"
        | "ARCHIVE_MALFORMED" => ApiError::PathTraversal,
        "ARCHIVE_TOO_LARGE" | "ARCHIVE_TOO_MANY_FILES" => ApiError::ArchiveTooLarge,
        "EXPIRED" | "PAIRING_EXPIRED" | "PAIRING_ALREADY_AUTHORIZED" => ApiError::Expired,
        "QUOTA_EXCEEDED" => ApiError::QuotaExceeded,
        "RATE_LIMITED" => ApiError::RateLimited(0),
        "INTERNAL_ERROR" => ApiError::Internal,
        _ => ApiError::Other {
            code: payload.code,
            message: payload.message,
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use reqwest::StatusCode;

    use super::*;

    fn payload(code: &str) -> ApiErrorPayload {
        ApiErrorPayload {
            code: code.to_owned(),
            message: format!("msg-for-{code}"),
        }
    }

    #[test]
    fn unauthorized_maps_to_unauthorized() {
        let e = from_envelope(payload("UNAUTHORIZED"), StatusCode::UNAUTHORIZED);
        assert!(matches!(e, ApiError::Unauthorized));
    }

    #[test]
    fn password_wrong_maps_to_unauthorized() {
        let e = from_envelope(payload("PASSWORD_WRONG"), StatusCode::UNAUTHORIZED);
        assert!(matches!(e, ApiError::Unauthorized));
    }

    #[test]
    fn forbidden_maps_to_forbidden() {
        let e = from_envelope(payload("FORBIDDEN"), StatusCode::FORBIDDEN);
        assert!(matches!(e, ApiError::Forbidden));
    }

    #[test]
    fn account_disabled_maps_to_forbidden() {
        let e = from_envelope(payload("ACCOUNT_DISABLED"), StatusCode::FORBIDDEN);
        assert!(matches!(e, ApiError::Forbidden));
    }

    #[test]
    fn not_found_maps_to_not_found() {
        let e = from_envelope(payload("NOT_FOUND"), StatusCode::NOT_FOUND);
        assert!(matches!(e, ApiError::NotFound));
    }

    #[test]
    fn validation_error_carries_message() {
        let e = from_envelope(
            payload("VALIDATION_ERROR"),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
        match e {
            ApiError::BadRequest(m) => assert_eq!(m, "msg-for-VALIDATION_ERROR"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn archive_path_traversal_maps_to_path_traversal() {
        let e = from_envelope(payload("ARCHIVE_PATH_TRAVERSAL"), StatusCode::BAD_REQUEST);
        assert!(matches!(e, ApiError::PathTraversal));
    }

    #[test]
    fn archive_symlink_maps_to_path_traversal() {
        let e = from_envelope(payload("ARCHIVE_SYMLINK"), StatusCode::BAD_REQUEST);
        assert!(matches!(e, ApiError::PathTraversal));
    }

    #[test]
    fn pairing_expired_maps_to_expired() {
        let e = from_envelope(payload("PAIRING_EXPIRED"), StatusCode::GONE);
        assert!(matches!(e, ApiError::Expired));
    }

    #[test]
    fn archive_too_large_maps_to_archive_too_large() {
        let e = from_envelope(payload("ARCHIVE_TOO_LARGE"), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(matches!(e, ApiError::ArchiveTooLarge));
    }

    #[test]
    fn archive_too_many_files_maps_to_archive_too_large() {
        let e = from_envelope(payload("ARCHIVE_TOO_MANY_FILES"), StatusCode::BAD_REQUEST);
        assert!(matches!(e, ApiError::ArchiveTooLarge));
    }

    #[test]
    fn quota_exceeded_maps_to_quota_exceeded() {
        let e = from_envelope(payload("QUOTA_EXCEEDED"), StatusCode::PAYMENT_REQUIRED);
        assert!(matches!(e, ApiError::QuotaExceeded));
    }

    #[test]
    fn rate_limited_starts_at_zero() {
        let e = from_envelope(payload("RATE_LIMITED"), StatusCode::TOO_MANY_REQUESTS);
        assert!(matches!(e, ApiError::RateLimited(0)));
    }

    #[test]
    fn internal_error_maps_to_internal() {
        let e = from_envelope(payload("INTERNAL_ERROR"), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(matches!(e, ApiError::Internal));
    }

    #[test]
    fn unknown_code_maps_to_other() {
        let e = from_envelope(payload("BRAND_NEW_CODE"), StatusCode::IM_A_TEAPOT);
        match e {
            ApiError::Other { code, message } => {
                assert_eq!(code, "BRAND_NEW_CODE");
                assert_eq!(message, "msg-for-BRAND_NEW_CODE");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn unauthorized_converts_to_auth() {
        let e: als_core::Error = ApiError::Unauthorized.into();
        assert!(matches!(e, als_core::Error::Auth));
    }

    #[test]
    fn quota_converts_to_quota() {
        let e: als_core::Error = ApiError::QuotaExceeded.into();
        assert!(matches!(e, als_core::Error::Quota));
    }

    #[test]
    fn rate_limited_preserves_seconds() {
        let e: als_core::Error = ApiError::RateLimited(42).into();
        match e {
            als_core::Error::RateLimited(n) => assert_eq!(n, 42),
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn internal_converts_to_server() {
        let e: als_core::Error = ApiError::Internal.into();
        assert!(matches!(e, als_core::Error::Server(_)));
    }

    #[test]
    fn other_converts_to_server() {
        let e: als_core::Error = ApiError::Other {
            code: "novel".into(),
            message: "what".into(),
        }
        .into();
        assert!(matches!(e, als_core::Error::Server(_)));
    }
}
