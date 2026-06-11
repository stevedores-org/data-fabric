//! Opaque cursor-based pagination for list endpoints.
//!
//! A cursor is the hex-encoded JSON of the last seen row's sort key. The shape
//! is intentionally opaque to clients — they must echo back whatever
//! `next_cursor` we returned without parsing it. Hex keeps the cursor
//! URL-safe and doesn't need a new crate dependency (`hex` is already pulled in
//! for IDs).
//!
//! For `/v1/runs` the sort key is `(created_at DESC, id DESC)`, so the cursor
//! carries both fields. Including `id` breaks ties when two runs share the
//! same `created_at`.
//!
//! `/v1/policies/rules` and `/v1/agents` follow the same shape: the cursor
//! carries the row's primary sort field plus `id` as a tiebreaker. All cursor
//! types use single-letter serde keys (`c`, `i`, `n`, `p`) to keep the
//! hex-encoded payload short — clients still get a single opaque blob.

use serde::{Deserialize, Serialize};
use worker::Result;

/// Sort-key tuple for `/v1/runs` pagination.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunsCursor {
    /// Last row's `created_at` (ISO 8601 string from D1).
    #[serde(rename = "c")]
    pub created_at: String,
    /// Last row's `id`, used as a tiebreaker for rows sharing `created_at`.
    #[serde(rename = "i")]
    pub id: String,
}

impl RunsCursor {
    /// Encode the cursor as a hex string suitable for `?cursor=` query params.
    pub fn encode(&self) -> Result<String> {
        let json = serde_json::to_vec(self)
            .map_err(|e| worker::Error::RustError(format!("encode cursor: {e}")))?;
        Ok(hex::encode(json))
    }

    /// Decode a hex-encoded cursor. Returns `Ok(None)` if the input is empty,
    /// `Err` if the input is present but malformed (callers should map this
    /// to `INVALID_CURSOR` 400).
    pub fn decode(raw: Option<&str>) -> Result<Option<Self>> {
        let Some(raw) = raw.filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let bytes = hex::decode(raw)
            .map_err(|e| worker::Error::RustError(format!("decode cursor hex: {e}")))?;
        let cursor: Self = serde_json::from_slice(&bytes)
            .map_err(|e| worker::Error::RustError(format!("decode cursor json: {e}")))?;
        Ok(Some(cursor))
    }
}

/// Sort-key tuple for `/v1/policies/rules` pagination.
///
/// Rows are ordered by `(priority DESC, created_at ASC, id ASC)`. Carrying all
/// three fields lets the query resume deterministically even when many rules
/// share the same `priority` and `created_at` (common when seeded in batches).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyRulesCursor {
    /// Last row's `priority`.
    #[serde(rename = "p")]
    pub priority: i32,
    /// Last row's `created_at` (ISO 8601 string from D1).
    #[serde(rename = "c")]
    pub created_at: String,
    /// Last row's `id`, used as a final tiebreaker.
    #[serde(rename = "i")]
    pub id: String,
}

impl PolicyRulesCursor {
    pub fn encode(&self) -> Result<String> {
        let json = serde_json::to_vec(self)
            .map_err(|e| worker::Error::RustError(format!("encode cursor: {e}")))?;
        Ok(hex::encode(json))
    }

    pub fn decode(raw: Option<&str>) -> Result<Option<Self>> {
        let Some(raw) = raw.filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let bytes = hex::decode(raw)
            .map_err(|e| worker::Error::RustError(format!("decode cursor hex: {e}")))?;
        let cursor: Self = serde_json::from_slice(&bytes)
            .map_err(|e| worker::Error::RustError(format!("decode cursor json: {e}")))?;
        Ok(Some(cursor))
    }
}

/// Sort-key tuple for `/v1/agents` pagination.
///
/// Rows are ordered by `(name ASC, id ASC)`. Carrying `id` breaks ties for
/// agents that share a name (rare but possible across capability variants).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentsCursor {
    /// Last row's `name`.
    #[serde(rename = "n")]
    pub name: String,
    /// Last row's `id`, used as a tiebreaker.
    #[serde(rename = "i")]
    pub id: String,
}

impl AgentsCursor {
    pub fn encode(&self) -> Result<String> {
        let json = serde_json::to_vec(self)
            .map_err(|e| worker::Error::RustError(format!("encode cursor: {e}")))?;
        Ok(hex::encode(json))
    }

    pub fn decode(raw: Option<&str>) -> Result<Option<Self>> {
        let Some(raw) = raw.filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let bytes = hex::decode(raw)
            .map_err(|e| worker::Error::RustError(format!("decode cursor hex: {e}")))?;
        let cursor: Self = serde_json::from_slice(&bytes)
            .map_err(|e| worker::Error::RustError(format!("decode cursor json: {e}")))?;
        Ok(Some(cursor))
    }
}

/// Clamp a client-supplied `?limit=` to the documented bounds.
///
/// Default 50, hard max 200 — matches what the existing handler already did
/// before pagination was added, so behavior is unchanged for callers that
/// don't send `?cursor=`.
pub const DEFAULT_PAGE_LIMIT: u32 = 50;
pub const MAX_PAGE_LIMIT: u32 = 200;

pub fn clamp_limit(raw: Option<u32>) -> u32 {
    raw.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrips_through_hex() {
        let cursor = RunsCursor {
            created_at: "2026-05-13T10:00:00Z".into(),
            id: "abc123".into(),
        };
        let encoded = cursor.encode().expect("encode");
        let decoded = RunsCursor::decode(Some(&encoded))
            .expect("decode")
            .expect("some");
        assert_eq!(decoded, cursor);
    }

    #[test]
    fn decode_returns_none_for_absent_or_empty() {
        assert!(RunsCursor::decode(None).unwrap().is_none());
        assert!(RunsCursor::decode(Some("")).unwrap().is_none());
    }

    #[test]
    fn decode_errors_on_garbage() {
        assert!(RunsCursor::decode(Some("not-hex-zzzz")).is_err());
        // Valid hex but not JSON
        assert!(RunsCursor::decode(Some("deadbeef")).is_err());
    }

    #[test]
    fn clamp_limit_applies_defaults_and_caps() {
        assert_eq!(clamp_limit(None), DEFAULT_PAGE_LIMIT);
        assert_eq!(clamp_limit(Some(10)), 10);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(5_000)), MAX_PAGE_LIMIT);
    }

    // ── PolicyRulesCursor ──────────────────────────────────────

    #[test]
    fn policy_rules_cursor_roundtrips_through_hex() {
        let cursor = PolicyRulesCursor {
            priority: 100,
            created_at: "2026-05-13T10:00:00Z".into(),
            id: "rule-abc".into(),
        };
        let encoded = cursor.encode().expect("encode");
        let decoded = PolicyRulesCursor::decode(Some(&encoded))
            .expect("decode")
            .expect("some");
        assert_eq!(decoded, cursor);
    }

    #[test]
    fn policy_rules_cursor_decode_returns_none_for_absent_or_empty() {
        assert!(PolicyRulesCursor::decode(None).unwrap().is_none());
        assert!(PolicyRulesCursor::decode(Some("")).unwrap().is_none());
    }

    #[test]
    fn policy_rules_cursor_decode_errors_on_garbage() {
        assert!(PolicyRulesCursor::decode(Some("not-hex-zzzz")).is_err());
        assert!(PolicyRulesCursor::decode(Some("deadbeef")).is_err());
    }

    // ── AgentsCursor ───────────────────────────────────────────

    #[test]
    fn agents_cursor_roundtrips_through_hex() {
        let cursor = AgentsCursor {
            name: "agent-orchestrator".into(),
            id: "ag-42".into(),
        };
        let encoded = cursor.encode().expect("encode");
        let decoded = AgentsCursor::decode(Some(&encoded))
            .expect("decode")
            .expect("some");
        assert_eq!(decoded, cursor);
    }

    #[test]
    fn agents_cursor_decode_returns_none_for_absent_or_empty() {
        assert!(AgentsCursor::decode(None).unwrap().is_none());
        assert!(AgentsCursor::decode(Some("")).unwrap().is_none());
    }

    #[test]
    fn agents_cursor_decode_errors_on_garbage() {
        assert!(AgentsCursor::decode(Some("not-hex-zzzz")).is_err());
        assert!(AgentsCursor::decode(Some("deadbeef")).is_err());
    }
}
