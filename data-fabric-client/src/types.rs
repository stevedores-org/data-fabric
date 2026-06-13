use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Entity status enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Created,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

// Entity kind discriminator
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

// Run types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateRun {
    pub repo: String,
    pub trigger: Option<String>,
    pub actor: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Created {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AivcsPullRequest {
    pub id: String,
    pub repo: String,
    pub run_id: String,
    pub title: String,
    pub status: String,
    pub source_branch: Option<String>,
    pub target_branch: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub change_set: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

// WS2 Task types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateTask {
    pub run_id: String,
    pub plan_id: Option<String>,
    pub name: String,
    pub actor: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

// M1 Agent Task types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskCreated {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskCompleteRequest {
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskFailRequest {
    pub error: String,
}

// Agent registration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegisterAgent {
    pub name: String,
    pub capabilities: Vec<String>,
    pub endpoint: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub endpoint: Option<String>,
    pub last_heartbeat: Option<String>,
    pub status: String,
    pub metadata: Option<serde_json::Value>,
}

// Checkpoints
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateCheckpoint {
    pub thread_id: String,
    pub node_id: String,
    pub parent_id: Option<String>,
    pub state: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckpointCreated {
    pub id: String,
    pub thread_id: String,
    pub state_r2_key: String,
}

// Artifact Response
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactStoredResponse {
    pub key: String,
    pub scoped_key: String,
    pub size: usize,
    pub stored: bool,
}

// Policy types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyCheckRequest {
    pub action: String,
    pub actor: String,
    pub resource: Option<String>,
    pub context: Option<serde_json::Value>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyCheckResponse {
    pub id: String,
    pub action: String,
    pub decision: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limited: Option<bool>,
}

// Pilot Metrics types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PilotMetrics {
    pub window: String,
    pub window_seconds: i64,
    pub sample_counts: SampleCounts,
    pub kpis: Kpis,
    #[serde(default)]
    pub null_reasons: BTreeMap<String, String>,
    pub meta: Meta,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SampleCounts {
    pub tasks: i64,
    pub events: i64,
    pub decisions: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Kpis {
    pub task_completion_rate: Option<f64>,
    pub mttr_p50_seconds: Option<f64>,
    pub mttr_p95_seconds: Option<f64>,
    pub context_reuse_rate: Option<f64>,
    pub human_intervention_rate: Option<f64>,
    pub event_throughput_per_sec: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    pub generated_at: String,
    pub tenant_id: String,
}

// Health Response
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthResponse {
    pub service: String,
    pub status: String,
    pub mission: String,
}

// Integration types (for WS6 intake endpoints)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationTarget {
    Oxidizedgraph,
    Aivcs,
    LlamaRs,
    Mom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphExecBatch {
    pub graph_id: String,
    pub thread_id: String,
    pub events: Vec<GraphExecEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphExecEvent {
    pub event_type: GraphExecEventType,
    pub node_id: String,
    pub node_type: Option<String>,
    pub parent_node_id: Option<String>,
    pub state: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphExecEventType {
    NodeStart,
    NodeEnd,
    NodeError,
    EdgeTraversal,
    CheckpointSave,
    CheckpointRestore,
    GraphStart,
    GraphEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineEvent {
    pub pipeline_id: String,
    pub repo: String,
    pub event_type: PipelineEventType,
    pub commit_sha: Option<String>,
    pub branch: Option<String>,
    pub actor: String,
    pub artifacts: Option<Vec<PipelineArtifact>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineEventType {
    PipelineStart,
    PipelineEnd,
    StageStart,
    StageEnd,
    TestResult,
    BuildComplete,
    DeployStart,
    DeployEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineArtifact {
    pub key: String,
    pub content_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InferenceRequest {
    pub model: String,
    pub prompt: Option<String>,
    pub messages: Option<serde_json::Value>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub tools: Option<Vec<String>>,
    pub run_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InferenceTelemetry {
    pub task_id: String,
    pub event_type: InferenceTelemetryType,
    pub model: String,
    pub tokens_in: Option<u32>,
    pub tokens_out: Option<u32>,
    pub duration_ms: Option<u64>,
    pub tool_calls: Option<Vec<InferenceToolCall>>,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InferenceTelemetryType {
    InferenceStart,
    InferenceEnd,
    ToolUse,
    TokenStream,
    InferenceError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InferenceToolCall {
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextRequest {
    pub query: String,
    pub model: Option<String>,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    pub top_k: Option<usize>,
    pub token_budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextPackResponse {
    pub query_id: String,
    pub latency_ms: i64,
    pub token_budget: usize,
    pub used_tokens: usize,
    pub dropped_due_to_budget: usize,
    pub items: Vec<ContextItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextItem {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub score: f64,
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEvent {
    pub run_id: Option<String>,
    pub thread_id: Option<String>,
    pub event_type: String,
    pub node_id: Option<String>,
    pub actor: Option<String>,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEventBatch {
    pub events: Vec<GraphEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEventAck {
    pub accepted: usize,
    pub queued: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
}
