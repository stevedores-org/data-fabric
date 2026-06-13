use serde::{Deserialize, Serialize};
use super::reasoning::TokenCost;

#[allow(dead_code)]
pub const REASONING_TRACE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct TelemetrySnapshot {
    pub agent_name: String,
    pub agent_type: String,
    pub status: String,
    pub duration_seconds: u32,
    pub total_attempts: u32,
    pub success_rate: f32,
    pub namespace: String,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TelemetryAck {
    pub id: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub struct CreateReasoningTrace {
    #[serde(default = "default_reasoning_trace_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    pub agent_id: String,
    pub job_id: String,
    #[serde(default)]
    pub parent_span_id: Option<String>,
    pub step_number: u32,
    pub step_type: String,
    #[serde(default)]
    pub inputs: Option<serde_json::Value>,
    #[serde(default)]
    pub outputs: Option<serde_json::Value>,
    #[serde(default)]
    pub token_cost: TokenCost,
    pub started_at: String,
    pub completed_at: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[allow(dead_code)]
impl CreateReasoningTrace {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.schema_version == 0 {
            return Err("schema_version must be greater than zero".into());
        }
        if self.agent_id.trim().is_empty() {
            return Err("agent_id is required".into());
        }
        if self.job_id.trim().is_empty() {
            return Err("job_id is required".into());
        }
        if self.step_type.trim().is_empty() {
            return Err("step_type is required".into());
        }
        if self.started_at.trim().is_empty() {
            return Err("started_at is required".into());
        }
        if self.completed_at.trim().is_empty() {
            return Err("completed_at is required".into());
        }
        Ok(())
    }
}

#[allow(dead_code)]
fn default_reasoning_trace_schema_version() -> u32 {
    REASONING_TRACE_SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct TracePayloadStorage {
    pub inline: Option<serde_json::Value>,
    pub archive_url: Option<String>,
    pub size_bytes: u64,
}

#[allow(dead_code)]
impl TracePayloadStorage {
    pub fn empty() -> Self {
        Self {
            inline: None,
            archive_url: None,
            size_bytes: 0,
        }
    }

    pub fn is_archived(&self) -> bool {
        self.archive_url.is_some()
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct ReasoningTraceRecord {
    pub id: String,
    pub schema_version: u32,
    pub idempotency_key: String,
    pub agent_id: String,
    pub job_id: String,
    pub parent_span_id: Option<String>,
    pub step_number: u32,
    pub step_type: String,
    pub inputs: TracePayloadStorage,
    pub outputs: TracePayloadStorage,
    pub token_cost: TokenCost,
    pub started_at: String,
    pub completed_at: String,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub struct ReasoningTraceAck {
    pub id: String,
    pub accepted: bool,
    pub duplicate: bool,
    pub schema_version: u32,
    pub archived_inputs: bool,
    pub archived_outputs: bool,
}
