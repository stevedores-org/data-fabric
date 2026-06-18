//! AIVCS operational concepts — CMDB (monitored web properties / endpoints) and
//! the first-class `incident` projection. Mirrors the SQL tables in migration
//! `0022_aivcs_cmdb_and_incident.sql`: read models (serialized to API consumers)
//! plus write-request bodies. The sentinel reads enabled endpoints from the CMDB
//! and upserts incidents here; `sre-agent` / `ciso-agent` query `incident`.

use serde::{Deserialize, Serialize};

// ── CMDB read models ────────────────────────────────────────────

/// A monitored web property (configuration item), e.g. `lornu.ai`.
#[derive(Debug, Clone, Serialize)]
pub struct CmdbProperty {
    pub id: String,
    pub domain: String,
    pub name: Option<String>,
    pub brand_group: Option<String>,
    pub criticality: String,
    pub owner: Option<String>,
    pub enabled: bool,
    /// HITL provenance: `seed` | `human` | `agent`.
    pub source: String,
    pub updated_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A monitored endpoint under a property, e.g. `https://dfc.aivcs.io/health`.
#[derive(Debug, Clone, Serialize)]
pub struct CmdbEndpoint {
    pub id: String,
    pub property_id: String,
    pub url: String,
    pub method: String,
    pub check_type: String,
    pub expected_status: i64,
    pub latency_slo_ms: Option<i64>,
    pub enabled: bool,
    pub source: String,
    pub updated_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ── incident read model ─────────────────────────────────────────

/// A first-class AIVCS operational incident.
#[derive(Debug, Clone, Serialize)]
pub struct Incident {
    pub id: String,
    pub property_id: Option<String>,
    pub endpoint_id: Option<String>,
    pub dedup_key: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub severity: String,
    pub status: String,
    pub signal: Option<String>,
    pub detector: String,
    pub github_issue_number: Option<i64>,
    pub github_issue_url: Option<String>,
    pub detected_at: String,
    pub resolved_at: Option<String>,
    pub mttr_seconds: Option<i64>,
}

// ── write-request bodies ────────────────────────────────────────

/// `POST /v1/incidents` body. The sentinel sends `dedup_key` so the server (or
/// caller) keeps a single open incident per ongoing problem.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateIncident {
    pub property_id: Option<String>,
    pub endpoint_id: Option<String>,
    pub dedup_key: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    pub signal: Option<String>,
    #[serde(default = "default_detector")]
    pub detector: String,
    pub github_issue_number: Option<i64>,
    pub github_issue_url: Option<String>,
}

/// `PATCH /v1/incidents/:id` body. All fields optional; only provided fields
/// are written (the rest are preserved via SQL `COALESCE`).
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateIncident {
    pub status: Option<String>,
    pub severity: Option<String>,
    pub signal: Option<String>,
    pub github_issue_number: Option<i64>,
    pub github_issue_url: Option<String>,
    pub resolved_at: Option<String>,
    pub mttr_seconds: Option<i64>,
}

fn default_severity() -> String {
    "sev3".to_string()
}
fn default_detector() -> String {
    "uptime-sentinel".to_string()
}
