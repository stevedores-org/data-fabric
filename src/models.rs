use serde::{Deserialize, Serialize};

// ── Runs (WS2: core domain entity) ─────────────────────────────

#[derive(Deserialize)]
pub struct CreateRun {
    pub repo: String,
    pub trigger: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct RunCreated {
    pub id: String,
    pub status: String,
    pub repo: String,
}

// ── Provenance Events (WS3: append-only audit trail) ────────────

#[derive(Deserialize)]
pub struct ProvenanceEvent {
    pub run_id: String,
    pub event_type: String,
    pub actor: String,
    pub payload: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct EventAck {
    pub id: String,
    pub event_type: String,
    pub accepted: bool,
}

// ── Policy (WS4: governance layer) ──────────────────────────────

#[derive(Deserialize)]
pub struct PolicyCheckRequest {
    pub action: String,
    pub actor: String,
    pub resource: Option<String>,
    pub context: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct PolicyCheckResponse {
    pub action: String,
    pub decision: String,
    pub reason: String,
}
