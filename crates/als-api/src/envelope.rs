//! Wire-format envelope parsing.
//!
//! Every JSON response from als-api is wrapped in
//! `{ "success": true, "data": ... } | { "success": false, "error": { code, message } }`.
//! List endpoints add a `meta: { total, page, limit }` block alongside `data`.
//! Rather than rely on `serde(untagged)` plus phantom `True` / `False` types,
//! the envelope is parsed via a normal struct and the `success` discriminator
//! is validated post-hoc.

use reqwest::StatusCode;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::{ApiError, from_envelope};

/// The error half of the wire envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiErrorPayload {
    pub code: String,
    pub message: String,
}

/// Pagination metadata returned with list endpoints.
#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct PaginationMeta {
    pub total: u32,
    pub page: u32,
    pub limit: u32,
}

fn none_option<T>() -> Option<T> {
    None
}

#[derive(Deserialize)]
struct RawEnvelope<T> {
    success: bool,
    #[serde(default = "none_option")]
    data: Option<T>,
    #[serde(default = "none_option")]
    error: Option<ApiErrorPayload>,
}

#[derive(Deserialize)]
struct RawListEnvelope<T> {
    success: bool,
    #[serde(default = "Vec::new")]
    data: Vec<T>,
    #[serde(default = "none_option")]
    meta: Option<PaginationMeta>,
    #[serde(default = "none_option")]
    error: Option<ApiErrorPayload>,
}

#[derive(Deserialize)]
struct RawEnvelopeNoData {
    success: bool,
    #[serde(default = "none_option")]
    error: Option<ApiErrorPayload>,
}

/// Parse an envelope that carries a `data` payload of type `T`.
///
/// * If `success == true` → returns `data` as `T`.
/// * If `success == false` → builds an `ApiError` from the inner `error` block.
/// * Body is not a valid envelope (missing `success`, unparsable JSON,
///   `success == true` with no `data`, `success == false` with no `error`)
///   → `Decode`.
///
/// `status` is forwarded to `from_envelope` so future variants can branch
/// on the HTTP code if needed.
pub fn parse_envelope<T: DeserializeOwned>(body: &str, status: StatusCode) -> Result<T, ApiError> {
    let raw: RawEnvelope<T> =
        serde_json::from_str(body).map_err(|e| ApiError::Decode(format!("envelope: {e}")))?;
    if raw.success {
        raw.data
            .ok_or_else(|| ApiError::Decode("success=true but `data` field missing".into()))
    } else {
        let err = raw
            .error
            .ok_or_else(|| ApiError::Decode("success=false but `error` field missing".into()))?;
        Err(from_envelope(err, status))
    }
}

/// Parsed list-envelope response: items plus pagination metadata.
#[derive(Debug)]
pub struct ListPayload<T> {
    pub data: Vec<T>,
    pub meta: PaginationMeta,
}

/// Parse a list envelope with `data: [...]` plus `meta: { total, page, limit }`.
///
/// Required for `GET /api/sites` and any other paginated endpoint. On error
/// the failure path is identical to [`parse_envelope`].
pub fn parse_list_envelope<T: DeserializeOwned>(
    body: &str,
    status: StatusCode,
) -> Result<ListPayload<T>, ApiError> {
    let raw: RawListEnvelope<T> =
        serde_json::from_str(body).map_err(|e| ApiError::Decode(format!("envelope: {e}")))?;
    if raw.success {
        let meta = raw
            .meta
            .ok_or_else(|| ApiError::Decode("success=true list reply missing `meta`".into()))?;
        Ok(ListPayload {
            data: raw.data,
            meta,
        })
    } else {
        let err = raw
            .error
            .ok_or_else(|| ApiError::Decode("success=false but `error` field missing".into()))?;
        Err(from_envelope(err, status))
    }
}

/// Parse an envelope that carries no `data` payload (e.g. endpoints whose
/// success response is `{ "success": true }` only). 204 responses are handled
/// by the caller and never reach this function.
pub fn parse_envelope_no_data(body: &str, status: StatusCode) -> Result<(), ApiError> {
    let raw: RawEnvelopeNoData =
        serde_json::from_str(body).map_err(|e| ApiError::Decode(format!("envelope: {e}")))?;
    if raw.success {
        Ok(())
    } else {
        let err = raw
            .error
            .ok_or_else(|| ApiError::Decode("success=false but `error` field missing".into()))?;
        Err(from_envelope(err, status))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Payload {
        id: String,
        n: u32,
    }

    #[test]
    fn ok_payload_deserializes() {
        let body = r#"{"success":true,"data":{"id":"abc","n":7}}"#;
        let got: Payload = parse_envelope(body, StatusCode::OK).unwrap();
        assert_eq!(
            got,
            Payload {
                id: "abc".into(),
                n: 7
            }
        );
    }

    #[test]
    fn err_payload_yields_api_error() {
        let body = r#"{"success":false,"error":{"code":"NOT_FOUND","message":"nope"}}"#;
        let err = parse_envelope::<Payload>(body, StatusCode::NOT_FOUND).unwrap_err();
        assert!(matches!(err, ApiError::NotFound));
    }

    #[test]
    fn err_payload_with_unknown_code_routes_to_other() {
        let body = r#"{"success":false,"error":{"code":"weird","message":"hi"}}"#;
        let err = parse_envelope::<Payload>(body, StatusCode::IM_A_TEAPOT).unwrap_err();
        match err {
            ApiError::Other { code, message } => {
                assert_eq!(code, "weird");
                assert_eq!(message, "hi");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn missing_success_field_is_decode_error() {
        let body = r#"{"data":{"id":"abc","n":7}}"#;
        let err = parse_envelope::<Payload>(body, StatusCode::OK).unwrap_err();
        assert!(matches!(err, ApiError::Decode(_)));
    }

    #[test]
    fn invalid_json_is_decode_error() {
        let body = r"{not json";
        let err = parse_envelope::<Payload>(body, StatusCode::OK).unwrap_err();
        assert!(matches!(err, ApiError::Decode(_)));
    }

    #[test]
    fn ok_true_without_data_is_decode_error() {
        let body = r#"{"success":true}"#;
        let err = parse_envelope::<Payload>(body, StatusCode::OK).unwrap_err();
        match err {
            ApiError::Decode(msg) => assert!(msg.contains("data"), "msg = {msg}"),
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    #[test]
    fn ok_false_without_error_is_decode_error() {
        let body = r#"{"success":false}"#;
        let err = parse_envelope::<Payload>(body, StatusCode::INTERNAL_SERVER_ERROR).unwrap_err();
        match err {
            ApiError::Decode(msg) => assert!(msg.contains("error"), "msg = {msg}"),
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    #[test]
    fn ok_false_overrides_2xx_status() {
        // Servers must not lie like this, but if they do the envelope is
        // authoritative: the body says `success=false` so callers see an error.
        let body = r#"{"success":false,"error":{"code":"INTERNAL_ERROR","message":"oops"}}"#;
        let err = parse_envelope::<Payload>(body, StatusCode::OK).unwrap_err();
        assert!(matches!(err, ApiError::Internal));
    }

    #[test]
    fn no_data_ok_is_unit() {
        let body = r#"{"success":true}"#;
        parse_envelope_no_data(body, StatusCode::OK).unwrap();
    }

    #[test]
    fn no_data_err_yields_api_error() {
        let body = r#"{"success":false,"error":{"code":"QUOTA_EXCEEDED","message":"too many"}}"#;
        let err = parse_envelope_no_data(body, StatusCode::PAYMENT_REQUIRED).unwrap_err();
        assert!(matches!(err, ApiError::QuotaExceeded));
    }

    #[test]
    fn no_data_missing_ok_is_decode_error() {
        let body = r#"{"error":{"code":"x","message":"y"}}"#;
        let err = parse_envelope_no_data(body, StatusCode::OK).unwrap_err();
        assert!(matches!(err, ApiError::Decode(_)));
    }

    #[test]
    fn rate_limited_envelope_starts_at_zero() {
        // Retry-After lives in the header; envelope parsing alone yields 0.
        // The client is responsible for patching the actual seconds.
        let body = r#"{"success":false,"error":{"code":"RATE_LIMITED","message":"slow"}}"#;
        let err = parse_envelope::<Payload>(body, StatusCode::TOO_MANY_REQUESTS).unwrap_err();
        match err {
            ApiError::RateLimited(s) => assert_eq!(s, 0),
            other => panic!("expected RateLimited(0), got {other:?}"),
        }
    }

    #[test]
    fn list_envelope_deserializes() {
        let body = r#"{
            "success": true,
            "data": [{"id":"a","n":1},{"id":"b","n":2}],
            "meta": {"total": 7, "page": 1, "limit": 50}
        }"#;
        let got = parse_list_envelope::<Payload>(body, StatusCode::OK).unwrap();
        assert_eq!(got.data.len(), 2);
        assert_eq!(got.meta.total, 7);
        assert_eq!(got.meta.page, 1);
        assert_eq!(got.meta.limit, 50);
    }

    #[test]
    fn list_envelope_propagates_error() {
        let body = r#"{"success":false,"error":{"code":"UNAUTHORIZED","message":"nope"}}"#;
        let err = parse_list_envelope::<Payload>(body, StatusCode::UNAUTHORIZED).unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized));
    }

    #[test]
    fn list_envelope_rejects_missing_meta() {
        let body = r#"{"success":true,"data":[]}"#;
        let err = parse_list_envelope::<Payload>(body, StatusCode::OK).unwrap_err();
        match err {
            ApiError::Decode(msg) => assert!(msg.contains("meta"), "msg = {msg}"),
            other => panic!("expected Decode, got {other:?}"),
        }
    }
}
