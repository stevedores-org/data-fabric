use serde::{Deserialize, Serialize};

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
