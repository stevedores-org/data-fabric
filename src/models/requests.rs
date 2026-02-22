use serde::{Deserialize, Serialize};

// ── API request/response types ──────────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateRun {
    pub repo: String,
    pub trigger: Option<String>,
    pub actor: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateTask {
    pub run_id: String,
    pub plan_id: Option<String>,
    pub name: String,
    pub actor: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreatePlan {
    pub run_id: String,
    pub name: String,
    pub task_ids: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RecordToolCall {
    pub run_id: String,
    pub task_id: Option<String>,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct IngestEvent {
    pub run_id: String,
    pub event_type: String,
    pub actor: String,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyCheckRequest {
    pub action: String,
    pub actor: String,
    pub resource: Option<String>,
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateRelease {
    pub repo: String,
    pub version: String,
    pub run_id: String,
    pub artifact_ids: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

// ── Generic response envelope ───────────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Created {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EventAck {
    pub id: String,
    pub event_type: String,
    pub accepted: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyCheckResponse {
    pub action: String,
    pub decision: String,
    pub reason: String,
}
