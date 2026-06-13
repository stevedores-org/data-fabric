//! AIVCS — Slice 1: native `change_set` projection.
//!
//! Per issue #148 ("AIVCS UI as a human-facing control plane on top of
//! data-fabric"), AIVCS needs an entity above "diff artifact" that
//! represents a proposed change with metadata (status, risk, confidence,
//! and pointers to the diff/summary artifacts in R2).
//!
//! This module is intentionally narrow:
//!   • [`ChangeSet`]       — the read shape returned to callers / serialized
//!                           back from the D1 row.
//!   • [`CreateChangeSet`] — the creation payload accepted by the DB layer.
//!                           `id` is auto-generated if omitted (see
//!                           `db::create_change_set`).
//!   • [`ChangeSetStatus`] — typed enum for the `status` column, with the
//!                           five canonical lifecycle states.
//!
//! `RiskLevel` is *not* re-defined here; it already lives in [`crate::policy`]
//! with the same snake_case serde shape. We re-export it so callers using
//! `models::*` get a coherent surface without having to reach into the
//! policy module.
//!
//! ## Scope
//! Projection + types + DB layer only. HTTP routes and gold read-models
//! land in subsequent AIVCS slices.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

// Re-export the canonical RiskLevel so AIVCS callers can write
// `models::RiskLevel` instead of reaching into `crate::policy`.
pub use crate::policy::RiskLevel;

/// Lifecycle state of a [`ChangeSet`]. Mirrors the `status` column in the
/// `change_set` table (migration `0015_aivcs_change_set.sql`).
///
/// The transitions sketched by issue #148:
/// `Proposed → Reviewing → Approved → Merged`, with `Abandoned` as the
/// terminal "rejected/withdrawn" branch from any non-merged state.
#[allow(dead_code)] // HTTP route consumer lands in a later AIVCS slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSetStatus {
    Proposed,
    Reviewing,
    Approved,
    Merged,
    Abandoned,
}

#[allow(dead_code)] // HTTP route consumer lands in a later AIVCS slice.
impl ChangeSetStatus {
    /// Storage-string form of the status. Matches the values written by
    /// the SQL `DEFAULT 'proposed'` clause and by `db::create_change_set`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Reviewing => "reviewing",
            Self::Approved => "approved",
            Self::Merged => "merged",
            Self::Abandoned => "abandoned",
        }
    }
}

impl FromStr for ChangeSetStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "proposed" => Ok(Self::Proposed),
            "reviewing" => Ok(Self::Reviewing),
            "approved" => Ok(Self::Approved),
            "merged" => Ok(Self::Merged),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(format!("unknown change_set status: {other}")),
        }
    }
}

/// Read shape of a row in the `change_set` table. Returned by the DB layer
/// and serialized over the wire (snake_case JSON, matching the column
/// names in the migration).
#[allow(dead_code)] // HTTP route consumer lands in a later AIVCS slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub id: String,
    pub repo: String,
    pub base_ref: String,
    pub head_ref: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_agent_id: Option<String>,

    pub status: ChangeSetStatus,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<RiskLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_artifact_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_artifact_key: Option<String>,

    pub created_at: String,
}

/// Creation payload. `id` is optional — if omitted, the DB layer mints a
/// random hex id (same generator used by other entities). `status` defaults
/// to [`ChangeSetStatus::Proposed`] when omitted, matching the SQL DEFAULT.
#[allow(dead_code)] // HTTP route consumer lands in a later AIVCS slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateChangeSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    pub repo: String,
    pub base_ref: String,
    pub head_ref: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_agent_id: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ChangeSetStatus>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<RiskLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_artifact_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_artifact_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_set_status_round_trips_str_to_enum_to_str_for_all_variants() {
        // Every variant of the lifecycle enum must round-trip through its
        // storage-string form. If a new variant is added to the enum but the
        // as_str / FromStr branches are not updated, this test will fail —
        // catching it before a row hits D1 with an unmappable status.
        let all = [
            ChangeSetStatus::Proposed,
            ChangeSetStatus::Reviewing,
            ChangeSetStatus::Approved,
            ChangeSetStatus::Merged,
            ChangeSetStatus::Abandoned,
        ];
        for status in all {
            let s = status.as_str();
            let parsed = ChangeSetStatus::from_str(s).expect("known status must parse");
            assert_eq!(parsed, status, "round-trip failed for {s}");
            assert_eq!(parsed.as_str(), s);
        }
    }

    #[test]
    fn change_set_status_default_storage_string_is_proposed() {
        // The SQL migration sets DEFAULT 'proposed' for the status column.
        // If the enum's `Proposed` variant ever stops mapping to that
        // literal, freshly inserted rows would deserialize incorrectly.
        assert_eq!(ChangeSetStatus::Proposed.as_str(), "proposed");
        assert_eq!(
            ChangeSetStatus::from_str("proposed").unwrap(),
            ChangeSetStatus::Proposed,
        );
    }

    #[test]
    fn change_set_status_rejects_unknown_storage_value() {
        let err = ChangeSetStatus::from_str("rejected").expect_err("unknown variant must error");
        assert!(err.contains("rejected"), "error must mention the bad value");
    }

    #[test]
    fn change_set_round_trips_serde_json() {
        let cs = ChangeSet {
            id: "cs_abcdef".into(),
            repo: "acme/checkout-service".into(),
            base_ref: "main".into(),
            head_ref: "feature/optimize-checkout".into(),
            author_agent_id: Some("optimizer-7".into()),
            status: ChangeSetStatus::Reviewing,
            risk_level: Some(RiskLevel::Low),
            confidence: Some(0.85),
            run_id: Some("run_218".into()),
            diff_artifact_key: Some("aivcs/change_set/cs_abcdef/diff.patch".into()),
            summary_artifact_key: Some("aivcs/change_set/cs_abcdef/summary.md".into()),
            created_at: "2026-06-11T12:00:00.000Z".into(),
        };

        let json = serde_json::to_string(&cs).expect("serialize");
        let parsed: ChangeSet = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, cs);

        // Spot-check the wire shape — snake_case keys, status as a string,
        // and risk_level as a snake_case enum string (re-exported from
        // crate::policy::RiskLevel).
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["status"], "reviewing");
        assert_eq!(v["risk_level"], "low");
        assert_eq!(v["repo"], "acme/checkout-service");
    }

    #[test]
    fn change_set_round_trips_with_all_optional_fields_omitted() {
        // Minimum-viable shape — every optional field is None. Verifies
        // skip_serializing_if + default deserialization both work.
        let cs = ChangeSet {
            id: "cs_min".into(),
            repo: "acme/svc".into(),
            base_ref: "main".into(),
            head_ref: "feature/x".into(),
            author_agent_id: None,
            status: ChangeSetStatus::Proposed,
            risk_level: None,
            confidence: None,
            run_id: None,
            diff_artifact_key: None,
            summary_artifact_key: None,
            created_at: "2026-06-11T12:00:00.000Z".into(),
        };
        let json = serde_json::to_string(&cs).unwrap();
        // None fields must not appear in the serialized JSON.
        assert!(!json.contains("author_agent_id"));
        assert!(!json.contains("risk_level"));
        assert!(!json.contains("confidence"));
        assert!(!json.contains("run_id"));
        assert!(!json.contains("diff_artifact_key"));
        assert!(!json.contains("summary_artifact_key"));

        let parsed: ChangeSet = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cs);
    }

    #[test]
    fn create_change_set_accepts_minimal_payload() {
        // The minimum a caller must supply: repo + base_ref + head_ref.
        // Everything else is optional and falls back to SQL defaults
        // (id auto-generated; status defaults to 'proposed').
        let json = r#"{
            "repo": "acme/svc",
            "base_ref": "main",
            "head_ref": "feature/x"
        }"#;
        let payload: CreateChangeSet = serde_json::from_str(json).expect("parse minimal");
        assert_eq!(payload.repo, "acme/svc");
        assert!(payload.id.is_none());
        assert!(payload.status.is_none());
        assert!(payload.risk_level.is_none());
        assert!(payload.confidence.is_none());
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CiCheckRun {
    pub id: String,
    pub change_set_id: String,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Branch {
    pub id: String,
    pub repo: String,
    pub name: String,
    pub head_sha: String,
    pub agent_owner: Option<String>,
    pub status: String,
    pub created_at: String,
}
