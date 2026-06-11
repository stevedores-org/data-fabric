//! Epic 3 / #111 — ADK BaseAgent reasoning-trace records.
//!
//! Sink-side schema for the trace stream the BaseAgent emits on every step.
//! Hot rows live in D1; inputs / outputs larger than [`INLINE_LIMIT_BYTES`]
//! are archived to R2 with a pointer URL stored alongside the row.

use serde::{Deserialize, Serialize};

/// Inline-vs-archive threshold (bytes). Payloads larger than this MUST be
/// offloaded to R2 by the ingest handler.
pub const INLINE_LIMIT_BYTES: usize = 1024;

/// Current trace schema version. Bumped on breaking changes; consumers
/// dispatch on this when replaying historical rows.
pub const TRACE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    ToolCall,
    Thought,
    Commit,
    Observation,
    Error,
    Other,
}

impl StepType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ToolCall => "tool_call",
            Self::Thought => "thought",
            Self::Commit => "commit",
            Self::Observation => "observation",
            Self::Error => "error",
            Self::Other => "other",
        }
    }

    /// Parse the storage-layer string back into the enum. Unknown values
    /// fall back to [`StepType::Other`] — the migration's CHECK constraint
    /// should prevent unknown values reaching this path, but a future schema
    /// version might add variants this code hasn't seen yet.
    pub fn from_storage_str(s: &str) -> Self {
        match s {
            "tool_call" => Self::ToolCall,
            "thought" => Self::Thought,
            "commit" => Self::Commit,
            "observation" => Self::Observation,
            "error" => Self::Error,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenCost {
    #[serde(default)]
    pub input: u32,
    #[serde(default)]
    pub output: u32,
    #[serde(default)]
    pub cached: u32,
}

/// Request body for `POST /v1/reasoning-traces`. The BaseAgent's TraceSink
/// builds one of these per step and fires it off without blocking the
/// agent's main work (failure handling is the client's concern).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IngestReasoningTrace {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    pub agent_id: String,
    pub job_id: String,

    #[serde(default)]
    pub parent_span_id: Option<String>,

    pub step_number: u32,
    pub step_type: StepType,

    /// Free-form inputs (e.g. tool args, prompt). Redact PII before sending.
    #[serde(default)]
    pub inputs: Option<serde_json::Value>,
    /// Free-form outputs (e.g. tool result, model response).
    #[serde(default)]
    pub outputs: Option<serde_json::Value>,

    #[serde(default)]
    pub tokens: TokenCost,

    pub started_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,

    /// Required. Deterministic per (job, step). Lets the sink dedupe retries.
    pub idempotency_key: String,
}

fn default_schema_version() -> u32 {
    TRACE_SCHEMA_VERSION
}

/// Response from `POST /v1/reasoning-traces`. `deduplicated = true` means
/// the idempotency_key was already seen — the client should treat it as
/// success, not retry.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceAck {
    pub id: String,
    pub accepted: bool,
    #[serde(default)]
    pub deduplicated: bool,
}

/// Row shape returned to readers (GET endpoints, replay tools). Mirrors
/// the D1 row; `*_r2_key` is set for any payload that was offloaded.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ReasoningTrace {
    pub id: String,
    pub schema_version: u32,
    pub agent_id: String,
    pub job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub step_number: u32,
    pub step_type: StepType,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs_inline: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs_r2_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs_inline: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs_r2_key: Option<String>,

    pub tokens: TokenCost,

    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    pub created_at: String,
}

/// Result of running the inline-vs-archive decision on a payload.
pub enum PayloadDisposition {
    /// Stays inline in the D1 row.
    Inline(serde_json::Value),
    /// Exceeded [`INLINE_LIMIT_BYTES`]; archive to this R2 key, bytes attached.
    Archive { key: String, bytes: Vec<u8> },
}

/// Decide whether a payload stays inline or gets archived. The caller is
/// responsible for actually writing `Archive` bytes to R2.
pub fn classify_payload(
    payload: &serde_json::Value,
    r2_prefix: &str,
    trace_id: &str,
    field: &str,
) -> PayloadDisposition {
    // Serializing a serde_json::Value cannot fail — unwrap surfaces any
    // future regression instead of silently writing an empty payload.
    let serialized = serde_json::to_vec(payload).expect("serde_json::Value always serializes");
    if serialized.len() <= INLINE_LIMIT_BYTES {
        PayloadDisposition::Inline(payload.clone())
    } else {
        let key = format!("{r2_prefix}reasoning_traces/{trace_id}/{field}.json");
        PayloadDisposition::Archive {
            key,
            bytes: serialized,
        }
    }
}

/// Strip the well-known PII fields ADK clients flag via `__pii__`. This is
/// a defensive backstop — clients SHOULD redact before sending, but the
/// sink must not store known-tainted values either way.
///
/// Recursively walks objects/arrays. A key listed in `__pii__` at any
/// object level is replaced with the string `"<redacted>"`.
pub fn redact_pii(payload: &mut serde_json::Value) {
    match payload {
        serde_json::Value::Object(map) => {
            let pii_keys: Vec<String> = map
                .get("__pii__")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            for key in &pii_keys {
                if map.contains_key(key) {
                    map.insert(key.clone(), serde_json::Value::String("<redacted>".into()));
                }
            }
            // Strip the marker itself so the stored row doesn't leak which
            // fields the client flagged as sensitive.
            map.remove("__pii__");
            for (_, v) in map.iter_mut() {
                redact_pii(v);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items.iter_mut() {
                redact_pii(v);
            }
        }
        _ => {}
    }
}
