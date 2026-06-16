//! Typed error taxonomy for repository operations.
//!
//! The fabric talks to two storage backends (D1 and R2) through several call
//! sites today (`src/db.rs`, `src/storage.rs`). Each call site translates
//! `worker::Error` into either an HTTP response or a string with ad-hoc
//! semantics. That makes it hard to write shared retry logic, hard to map
//! errors to consistent HTTP status codes at the edge, and hard to add
//! observability that doesn't double-count classes of failure.
//!
//! This module replaces that with a small closed enum whose variants carry
//! routing intent rather than implementation detail:
//!
//! | Variant       | HTTP | Retriable? | Example                             |
//! |---------------|-----:|:----------:|-------------------------------------|
//! | [`Auth`]      | 401  | no         | missing/invalid tenant credentials  |
//! | [`Permanent`] | 400  | no         | malformed request, bad bindings     |
//! | [`NotFound`]  | 404  | no         | unknown run id, missing R2 key      |
//! | [`Conflict`]  | 409  | no         | unique-key collision, lease taken   |
//! | [`Internal`]  | 500  | no         | unexpected; bug or invariant break  |
//! | [`Transient`] | 503  | **yes**   | D1 busy, R2 timeout, `internal_error` |
//!
//! `Internal` and `Transient` are deliberately separate: a planner bug that
//! produced bad SQL is *not* worth retrying, even though both surface as 5xx
//! to the caller. `is_transient()` is the single source of truth for the
//! retry helper — adding a new variant later only requires updating that
//! one predicate, not every retry call site.
//!
//! See [`crate::retry`] for the policy that consumes [`Error::is_transient`].

use thiserror::Error;

/// Repository-level error.
///
/// The string payload is purely diagnostic — it surfaces in logs and in the
/// 5xx error envelope when something goes wrong, but the *classification*
/// (variant) is what callers and the retry policy branch on. Don't pattern
/// match on the string.
#[derive(Debug, Error)]
pub enum Error {
    /// Retriable failure: the underlying call can be re-issued and may
    /// succeed. Use for D1 busy / locked, R2 timeouts, and `internal_error`
    /// responses from the Cloudflare control plane.
    #[error("transient: {0}")]
    Transient(String),

    /// Non-retriable client error: the request is wrong and won't succeed
    /// by re-issuing. Use for malformed input, schema-violating writes,
    /// missing bindings.
    #[error("permanent: {0}")]
    Permanent(String),

    /// Caller is unauthenticated or unauthorized for this resource.
    #[error("auth: {0}")]
    Auth(String),

    /// The requested key/row does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// A uniqueness/state constraint prevented the write. Includes lease
    /// races (e.g. another agent claimed the task first).
    #[error("conflict: {0}")]
    Conflict(String),

    /// Server-side bug or invariant violation. Distinct from `Transient`:
    /// retrying won't help.
    #[error("internal: {0}")]
    Internal(String),
}

impl Error {
    /// HTTP status code this error should map to at the edge.
    ///
    /// Matches the acceptance criteria in stevedores-org/data-fabric#53:
    /// 400 / 401 / 404 / 409 / 500 / 503.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Permanent(_) => 400,
            Self::Auth(_) => 401,
            Self::NotFound(_) => 404,
            Self::Conflict(_) => 409,
            Self::Internal(_) => 500,
            Self::Transient(_) => 503,
        }
    }

    /// Should the retry policy attempt this error again?
    ///
    /// **Only** `Transient` retries. `Internal` is excluded deliberately —
    /// see the module-level table for the reasoning.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }
}

/// Convert a `worker::Error` into a classified repository error.
///
/// The classification is conservative: we recognize the variants that the
/// Cloudflare runtime currently spells out as definitely-transient
/// (`InternalError`, `RateLimitExceeded`) and bucket everything else as
/// either `NotFound` (for the small set of unambiguous 404 cases) or
/// `Internal`. As we encounter new transient strings in the wild we extend
/// the substring sniffer in [`is_transient_message`].
impl From<worker::Error> for Error {
    fn from(e: worker::Error) -> Self {
        use worker::Error as W;
        match &e {
            // Cloudflare-coded transients. These come back with a stable
            // `code` from the runtime; we trust the classification.
            W::InternalError(msg) | W::RateLimitExceeded(msg) => Error::Transient(msg.clone()),
            W::DailyLimitExceeded(msg) => Error::Permanent(msg.clone()),

            // For BindingError we're sure: the worker is misconfigured.
            // Not retriable, not the user's fault, surfaces as 500.
            W::BindingError(msg) => Error::Internal(format!("binding: {msg}")),

            // Everything else: peek at the rendered string to catch known
            // transient phrases coming out of D1/R2 that aren't yet typed
            // by the worker crate (D1_ERROR busy / SQLITE_BUSY, R2 timeout
            // / internal_error). Fall through to Internal so we don't
            // silently retry things we shouldn't.
            _ => {
                let msg = e.to_string();
                if is_transient_message(&msg) {
                    Error::Transient(msg)
                } else if is_not_found_message(&msg) {
                    Error::NotFound(msg)
                } else {
                    Error::Internal(msg)
                }
            }
        }
    }
}

/// Substring sniff for Cloudflare-specific transient failures.
///
/// Not exhaustive — extend as new strings show up in incidents. The cost of
/// a false negative (we don't retry something we could have) is bounded;
/// the cost of a false positive (we retry a permanent error) is wasted
/// budget and possibly write amplification, so we err narrow.
pub(crate) fn is_transient_message(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("internal_error")
        || m.contains("internal error")
        || m.contains("d1_error: database is locked")
        || m.contains("sqlite_busy")
        || m.contains("database is locked")
        || m.contains("timeout")
        || m.contains("timed out")
        || m.contains("temporarily unavailable")
        || m.contains("service unavailable")
        || m.contains("503")
}

/// Substring sniff for "this key/row does not exist".
///
/// D1's `first()` returns `Ok(None)` for missing rows so callers don't hit
/// this path for the common case. We only end up here when the runtime
/// surfaces a `NotFound` through an error rather than an `Option`.
pub(crate) fn is_not_found_message(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("not found") || m.contains("no such") || m.contains("404")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_code_covers_acceptance_matrix() {
        // Acceptance criterion: 400, 401, 404, 409, 500, 503 all reachable.
        assert_eq!(Error::Permanent("x".into()).status_code(), 400);
        assert_eq!(Error::Auth("x".into()).status_code(), 401);
        assert_eq!(Error::NotFound("x".into()).status_code(), 404);
        assert_eq!(Error::Conflict("x".into()).status_code(), 409);
        assert_eq!(Error::Internal("x".into()).status_code(), 500);
        assert_eq!(Error::Transient("x".into()).status_code(), 503);
    }

    #[test]
    fn only_transient_is_retriable() {
        // `is_transient` is what the retry helper branches on; if this
        // predicate ever returns true for anything else we'd retry
        // non-idempotent failures. Lock the set down explicitly.
        assert!(Error::Transient("busy".into()).is_transient());
        assert!(!Error::Permanent("bad".into()).is_transient());
        assert!(!Error::Auth("nope".into()).is_transient());
        assert!(!Error::NotFound("gone".into()).is_transient());
        assert!(!Error::Conflict("race".into()).is_transient());
        assert!(!Error::Internal("bug".into()).is_transient());
    }

    #[test]
    fn transient_sniffer_catches_cf_phrases() {
        assert!(is_transient_message("D1_ERROR: database is locked"));
        assert!(is_transient_message("SQLITE_BUSY"));
        assert!(is_transient_message("Request timed out"));
        assert!(is_transient_message("Internal error from R2"));
        assert!(is_transient_message("503 Service Unavailable"));
        assert!(is_transient_message("temporarily unavailable"));

        assert!(!is_transient_message("malformed JSON"));
        assert!(!is_transient_message("no such column: foo"));
    }

    #[test]
    fn not_found_sniffer_matches() {
        assert!(is_not_found_message("Object not found"));
        assert!(is_not_found_message("404 Not Found"));
        assert!(is_not_found_message("no such file"));
        assert!(!is_not_found_message("permission denied"));
    }

    #[test]
    fn display_includes_classification_and_message() {
        // The `#[error("transient: {0}")]` prefix is load-bearing for
        // log greppability. If someone "tidies" it away tests will fail.
        let e = Error::Transient("D1 busy".into());
        assert_eq!(format!("{e}"), "transient: D1 busy");
        let e = Error::NotFound("run abc".into());
        assert_eq!(format!("{e}"), "not found: run abc");
    }
}
