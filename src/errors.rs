//! Structured error envelope returned by the fabric API.
//!
//! All error responses share this shape so SDK/CLI/operator tools can rely on
//! a stable contract instead of parsing free-form text out of `Response::error`.
//!
//! ```json
//! { "error": { "code": "RUN_NOT_FOUND", "message": "run not found" } }
//! ```
//!
//! `code` is a stable, machine-readable identifier (SCREAMING_SNAKE_CASE).
//! `message` is human-readable and may change. Populate `details` when callers
//! need structured context (e.g. which fields failed validation).

use serde::Serialize;
use serde_json::Value;
use worker::{Response, Result};

#[derive(Serialize)]
pub struct ErrorEnvelope<'a> {
    pub code: &'a str,
    pub message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorEnvelope<'a>,
}

/// Build a JSON error response with a stable `{ "error": { code, message } }`
/// envelope and the given HTTP status.
pub fn error_response(code: &str, message: &str, status: u16) -> Result<Response> {
    let body = ErrorBody {
        error: ErrorEnvelope {
            code,
            message,
            details: None,
        },
    };
    Ok(Response::from_json(&body)?.with_status(status))
}

/// Build a JSON error response that includes structured details.
#[allow(dead_code)] // surface for future handlers (validation, conflict, etc.)
pub fn error_response_with_details(
    code: &str,
    message: &str,
    details: Value,
    status: u16,
) -> Result<Response> {
    let body = ErrorBody {
        error: ErrorEnvelope {
            code,
            message,
            details: Some(details),
        },
    };
    Ok(Response::from_json(&body)?.with_status(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_serializes_with_stable_shape() {
        let body = ErrorBody {
            error: ErrorEnvelope {
                code: "RUN_NOT_FOUND",
                message: "run not found",
                details: None,
            },
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["error"]["code"], "RUN_NOT_FOUND");
        assert_eq!(json["error"]["message"], "run not found");
        assert!(json["error"].get("details").is_none());
    }

    #[test]
    fn envelope_with_details_round_trips() {
        let details = serde_json::json!({ "fields": ["repo"] });
        let body = ErrorBody {
            error: ErrorEnvelope {
                code: "INVALID_REQUEST",
                message: "missing required fields",
                details: Some(details.clone()),
            },
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["error"]["details"], details);
    }
}
