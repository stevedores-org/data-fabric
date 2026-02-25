use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Checkpoint,
    Artifact,
    Decision,
    Context,
    RunSummary,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct UpsertMemoryItemRequest {
    pub repo: String,
    pub kind: MemoryKind,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    pub checkpoint_id: Option<String>,
    pub artifact_key: Option<String>,
    pub title: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub content_ref: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub success_rate: Option<f64>,
    pub source_created_at: Option<String>,
    pub ttl_seconds: Option<i64>,
    pub unsafe_reason: Option<String>,
    pub conflict_key: Option<String>,
    pub conflict_version: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryItemCreated {
    pub id: String,
    pub status: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct RetrieveMemoryRequest {
    pub repo: String,
    pub query: String,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub related_repos: Vec<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub include_stale: bool,
    #[serde(default)]
    pub include_unsafe: bool,
    #[serde(default)]
    pub include_conflicted: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct ContextPackRequest {
    #[serde(flatten)]
    pub retrieval: RetrieveMemoryRequest,
    #[serde(default = "default_token_budget")]
    pub token_budget: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct MemoryCandidate {
    pub id: String,
    pub repo: String,
    pub kind: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    pub title: Option<String>,
    pub summary: String,
    pub tags: Vec<String>,
    pub content_ref: Option<String>,
    pub success_rate: Option<f64>,
    pub stale: bool,
    pub unsafe_reason: Option<String>,
    pub conflicted: bool,
    pub estimated_tokens: usize,
    pub score: f64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RetrieveMemoryResponse {
    pub query_id: String,
    pub latency_ms: i64,
    pub total_candidates: usize,
    pub returned: usize,
    pub stale_filtered: usize,
    pub unsafe_filtered: usize,
    pub conflict_filtered: usize,
    pub items: Vec<MemoryCandidate>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ContextPackResponse {
    pub query_id: String,
    pub latency_ms: i64,
    pub token_budget: usize,
    pub used_tokens: usize,
    pub dropped_due_to_budget: usize,
    pub items: Vec<MemoryCandidate>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RetireMemoryResponse {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct MemoryGcRequest {
    #[serde(default = "default_gc_limit")]
    pub limit: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryGcResponse {
    pub scanned: usize,
    pub retired: usize,
    pub deleted: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RetrievalFeedback {
    pub query_id: String,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub success: bool,
    pub first_pass_success: bool,
    pub cache_hit: bool,
    pub latency_ms: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RetrievalFeedbackAck {
    pub recorded: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryEvalSummary {
    pub total_queries: i64,
    pub cache_hit_rate: f64,
    pub success_rate: f64,
    pub first_pass_success_rate: f64,
    pub p50_latency_ms: Option<i64>,
    pub p95_latency_ms: Option<i64>,
}

fn default_top_k() -> usize {
    8
}

fn default_token_budget() -> usize {
    4096
}

fn default_gc_limit() -> usize {
    1000
}

// ── Legacy Memory (migrated from mcp.rs) ─────────────────────

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
