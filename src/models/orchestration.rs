use serde::{Deserialize, Serialize};

// ── Agent Task Queue (M1) ───────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateAgentTask {
    pub job_id: String,
    pub task_type: String,
    #[serde(default)]
    pub priority: i32,
    pub params: Option<serde_json::Value>,
    pub graph_ref: Option<String>,
    pub play_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub max_retries: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentTask {
    pub id: String,
    pub job_id: String,
    pub task_type: String,
    pub priority: i32,
    pub status: String,
    pub params: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub agent_id: Option<String>,
    pub graph_ref: Option<String>,
    pub play_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub retry_count: i32,
    pub max_retries: i32,
    pub lease_expires_at: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskCreated {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskCompleteRequest {
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskFailRequest {
    pub error: String,
}

// ── Agent Registration (M1) ─────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RegisterAgent {
    pub name: String,
    pub capabilities: Vec<String>,
    pub endpoint: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub endpoint: Option<String>,
    pub last_heartbeat: Option<String>,
    pub status: String,
    pub metadata: Option<serde_json::Value>,
}

// ── Checkpoints (M2: oxidizedgraph state) ───────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateCheckpoint {
    pub thread_id: String,
    pub node_id: String,
    pub parent_id: Option<String>,
    pub state: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Checkpoint {
    pub id: String,
    pub thread_id: String,
    pub node_id: String,
    pub parent_id: Option<String>,
    pub state_r2_key: String,
    pub state_size_bytes: Option<i64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckpointCreated {
    pub id: String,
    pub thread_id: String,
    pub state_r2_key: String,
}

// ── Graph Events (M3: event pipeline) ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEvent {
    pub run_id: Option<String>,
    pub thread_id: Option<String>,
    pub event_type: String,
    pub node_id: Option<String>,
    pub actor: Option<String>,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct GraphEventBatch {
    pub events: Vec<GraphEvent>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct GraphEventAck {
    pub accepted: usize,
    pub queued: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
}

// ── Queue Envelope (WS3: issue #58) ─────────────────────────────

/// Envelope wrapping a GraphEvent with tenant context for queue transport.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct QueueEnvelope {
    pub tenant_id: String,
    pub event: GraphEvent,
}

// ── Gold Layer: Task Dependencies (WS3: issue #58) ──────────────

/// Edge in the task dependency graph (gold layer).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskDependencyEdge {
    pub run_id: String,
    pub task_id: String,
    pub depends_on_task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

// ── Replay Contract Stub (WS3: issue #58) ───────────────────────

/// Request to build a replay plan for a run.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ReplayPlanRequest {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_event_id: Option<String>,
}

/// A single step in a replay plan.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ReplayStep {
    pub sequence: usize,
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

/// Response containing the replay plan.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ReplayPlanResponse {
    pub run_id: String,
    pub steps: Vec<ReplayStep>,
    pub status: String,
}

// ── Common ─────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ── Trace / Provenance (WS3: issue #43) ───────────────────────

/// Single event in a trace slice (reconstructed execution narrative).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceEvent {
    pub id: String,
    pub run_id: Option<String>,
    pub thread_id: Option<String>,
    pub event_type: String,
    pub node_id: Option<String>,
    pub actor: Option<String>,
    pub payload: Option<serde_json::Value>,
    pub created_at: String,
}

/// Trace slice for a run: ordered events for debugging and replay.
/// When `?limit=` or `?hops=` is used, `total` and `truncated` indicate there may be more events (issue #61).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceResponse {
    pub run_id: String,
    pub events: Vec<TraceEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

// ── Provenance Links (WS3: causality chain) ─────────────────────

/// Single edge in a provenance/causality chain.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ProvenanceEdge {
    pub depth: i32,
    pub rel_type: String,
    pub from_kind: String,
    pub from_id: String,
    pub to_kind: String,
    pub to_id: String,
    pub relation: Option<String>,
    pub created_at: Option<String>,
}

/// Response for provenance chain query.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ProvenanceResponse {
    pub entity_kind: String,
    pub entity_id: String,
    pub direction: String,
    pub hops: u32,
    pub edges: Vec<ProvenanceEdge>,
}

// ── Gold Layer: Run Summaries (WS3) ─────────────────────────────

/// Materialized run summary (gold layer).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RunSummary {
    pub run_id: String,
    pub event_count: i32,
    pub first_event_at: Option<String>,
    pub last_event_at: Option<String>,
    pub actors: Vec<String>,
    pub event_types: Vec<String>,
    pub updated_at: String,
}
