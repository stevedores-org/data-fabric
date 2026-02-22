use serde::{Deserialize, Serialize};

// ── API request/response types ──────────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateRun {
    pub repo: String,
    pub trigger: Option<String>,
    pub actor: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

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
    pub entity_kind: Option<String>,
    pub entity_id: Option<String>,
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
    pub id: String,
    pub action: String,
    pub decision: String,
    pub reason: String,
    pub risk_level: String,
    pub matched_rule: Option<String>,
}

// ── Policy rules CRUD ──────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreatePolicyRule {
    pub name: String,
    #[serde(default = "default_wildcard")]
    pub action_pattern: String,
    #[serde(default = "default_wildcard")]
    pub resource_pattern: String,
    #[serde(default = "default_wildcard")]
    pub actor_pattern: String,
    #[serde(default = "default_risk_level")]
    pub risk_level: String,
    pub verdict: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub priority: i32,
}

fn default_wildcard() -> String {
    "*".into()
}

fn default_risk_level() -> String {
    "read".into()
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyRuleResponse {
    pub id: String,
    pub name: String,
    pub action_pattern: String,
    pub resource_pattern: String,
    pub actor_pattern: String,
    pub risk_level: String,
    pub verdict: String,
    pub reason: String,
    pub priority: i32,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct UpdatePolicyRule {
    pub name: Option<String>,
    pub action_pattern: Option<String>,
    pub resource_pattern: Option<String>,
    pub actor_pattern: Option<String>,
    pub risk_level: Option<String>,
    pub verdict: Option<String>,
    pub reason: Option<String>,
    pub priority: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyDecisionResponse {
    pub id: String,
    pub action: String,
    pub actor: String,
    pub resource: Option<String>,
    pub decision: String,
    pub reason: String,
    pub created_at: String,
    pub context: Option<serde_json::Value>,
}

// ── WS8: Multi-tenant provisioning ─────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TenantProvisionRequest {
    pub tenant_id: String,
    pub display_name: String,
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub quota_runs_per_minute: i64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TenantProvisionResponse {
    pub tenant_id: String,
    pub status: String,
    pub provisioned_in_ms: i64,
}
