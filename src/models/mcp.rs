use serde::{Deserialize, Serialize};

// ── MCP Task Queue (M1) ────────────────────────────────────────

/// Request to create a new MCP task in the agent work queue.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateMcpTask {
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

/// A claimed MCP task with full lifecycle fields (lease, retry, agent).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct McpTask {
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
pub struct McpTaskCreated {
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

// ── Agents (M1) ────────────────────────────────────────────────

/// Request to register a new agent.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RegisterAgent {
    pub name: String,
    pub capabilities: Vec<String>,
    pub endpoint: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// A registered agent with heartbeat and status.
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

// ── Checkpoints (M2) ──────────────────────────────────────────

/// Request to create a checkpoint for oxidizedgraph state.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateCheckpoint {
    pub thread_id: String,
    pub node_id: String,
    pub parent_id: Option<String>,
    pub state: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
}

/// A persisted checkpoint (state stored in R2, metadata in D1).
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

// ── Graph Events (M3) ─────────────────────────────────────────

/// A single graph execution event (node start/end, edge traversal, etc).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
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
}

// ── Memory (WS5: #45) ──────────────────────────────────────────

/// Request to create a memory entry (index over runs/artifacts/checkpoints).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateMemory {
    pub thread_id: String,
    pub scope: String,
    pub key: String,
    pub ref_type: String,
    pub ref_id: String,
    pub run_id: Option<String>,
    pub expires_at: Option<String>,
}

/// A memory entry for retrieval and context packing.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub id: String,
    pub run_id: Option<String>,
    pub thread_id: String,
    pub scope: String,
    pub key: String,
    pub ref_type: String,
    pub ref_id: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryCreated {
    pub id: String,
    pub thread_id: String,
    pub scope: String,
    pub key: String,
}

// ── Common ─────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}
