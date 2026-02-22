use serde::{Deserialize, Serialize};

// ── Common types ────────────────────────────────────────────────

/// Status shared across Run, Task, and Plan state machines.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Created,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// Entity kind discriminator for relationship endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Run,
    Task,
    Plan,
    ToolCall,
    Artifact,
    PolicyDecision,
    Release,
    Event,
}

// ── Run ─────────────────────────────────────────────────────────

/// A run is the top-level execution unit — one CI run, one agent session, etc.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Run {
    pub id: String,
    pub repo: String,
    pub status: Status,
    pub trigger: Option<String>,
    pub actor: String,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<serde_json::Value>,
}

// ── Task ────────────────────────────────────────────────────────

/// A task is a discrete unit of work within a run.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub run_id: String,
    pub plan_id: Option<String>,
    pub name: String,
    pub status: Status,
    pub actor: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<serde_json::Value>,
}

// ── Plan ────────────────────────────────────────────────────────

/// A plan is an ordered sequence of tasks with a dependency DAG.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    pub id: String,
    pub run_id: String,
    pub name: String,
    pub status: Status,
    pub task_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<serde_json::Value>,
}

// ── ToolCall ────────────────────────────────────────────────────

/// A tool call records a single tool invocation by an agent.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub run_id: String,
    pub task_id: Option<String>,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub status: Status,
    pub duration_ms: Option<u64>,
    pub created_at: String,
}

// ── Artifact ────────────────────────────────────────────────────

/// An artifact is a typed blob produced or consumed during a run.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    pub id: String,
    pub run_id: String,
    pub key: String,
    pub content_type: Option<String>,
    pub size_bytes: u64,
    pub checksum: String,
    pub created_at: String,
    pub metadata: Option<serde_json::Value>,
}

// ── PolicyDecision ──────────────────────────────────────────────

/// A persistent record of a policy evaluation.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyDecision {
    pub id: String,
    pub run_id: Option<String>,
    pub action: String,
    pub actor: String,
    pub resource: Option<String>,
    pub decision: PolicyVerdict,
    pub reason: String,
    pub created_at: String,
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict {
    Allow,
    Deny,
    Escalate,
}

// ── Release ─────────────────────────────────────────────────────

/// A release is a versioned snapshot tied to a set of runs and artifacts.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub id: String,
    pub repo: String,
    pub version: String,
    pub run_id: String,
    pub artifact_ids: Vec<String>,
    pub status: Status,
    pub created_at: String,
    pub metadata: Option<serde_json::Value>,
}

// ── Provenance Event ────────────────────────────────────────────

/// An append-only event capturing any state change in the fabric.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub run_id: String,
    pub entity_kind: EntityKind,
    pub entity_id: String,
    pub event_type: String,
    pub actor: String,
    pub created_at: String,
    pub payload: Option<serde_json::Value>,
}
