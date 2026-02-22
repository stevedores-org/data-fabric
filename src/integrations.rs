//! WS6: Orchestration Integration — adapters for oxidizedgraph, aivcs, llama.rs.
//!
//! Each integration target has:
//! 1. A typed contract (ingest schema) defining what the external system sends.
//! 2. An adapter that translates the external format into canonical fabric entities.
//! 3. A registration record for tracking active integrations.

use serde::{Deserialize, Serialize};

// ── Integration registry ────────────────────────────────────────

/// Supported integration targets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationTarget {
    Oxidizedgraph,
    Aivcs,
    LlamaRs,
}

impl IntegrationTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Oxidizedgraph => "oxidizedgraph",
            Self::Aivcs => "aivcs",
            Self::LlamaRs => "llama_rs",
        }
    }
}

/// Registration record for an active integration.
#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Integration {
    pub id: String,
    pub target: IntegrationTarget,
    pub name: String,
    pub endpoint: Option<String>,
    pub api_version: String,
    pub status: String,
    pub config: Option<serde_json::Value>,
    pub created_at: String,
    pub last_seen_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RegisterIntegration {
    pub target: IntegrationTarget,
    pub name: String,
    pub endpoint: Option<String>,
    pub api_version: Option<String>,
    pub config: Option<serde_json::Value>,
}

// ── oxidizedgraph contract ──────────────────────────────────────

/// oxidizedgraph sends graph execution state via this contract.
/// Maps to: checkpoints (state snapshots) + graph-events (node lifecycle).
pub mod oxidizedgraph {
    use super::*;

    /// A batch of graph execution events from oxidizedgraph.
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    pub struct GraphExecBatch {
        pub graph_id: String,
        pub thread_id: String,
        pub events: Vec<GraphExecEvent>,
    }

    /// A single graph execution event.
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
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

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
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

    /// Adapt an oxidizedgraph batch into canonical fabric types.
    pub fn adapt_to_graph_events(batch: &GraphExecBatch) -> Vec<crate::models::GraphEvent> {
        batch
            .events
            .iter()
            .map(|evt| {
                let event_type = match evt.event_type {
                    GraphExecEventType::NodeStart => "node.start",
                    GraphExecEventType::NodeEnd => "node.end",
                    GraphExecEventType::NodeError => "node.error",
                    GraphExecEventType::EdgeTraversal => "edge.traversal",
                    GraphExecEventType::CheckpointSave => "checkpoint.save",
                    GraphExecEventType::CheckpointRestore => "checkpoint.restore",
                    GraphExecEventType::GraphStart => "graph.start",
                    GraphExecEventType::GraphEnd => "graph.end",
                };

                let mut payload = serde_json::Map::new();
                payload.insert(
                    "source".into(),
                    serde_json::Value::String("oxidizedgraph".into()),
                );
                payload.insert(
                    "graph_id".into(),
                    serde_json::Value::String(batch.graph_id.clone()),
                );
                if let Some(ref nt) = evt.node_type {
                    payload.insert("node_type".into(), serde_json::Value::String(nt.clone()));
                }
                if let Some(ref err) = evt.error {
                    payload.insert("error".into(), serde_json::Value::String(err.clone()));
                }
                if let Some(ms) = evt.duration_ms {
                    payload.insert("duration_ms".into(), serde_json::Value::Number(ms.into()));
                }
                if let Some(ref meta) = evt.metadata {
                    payload.insert("metadata".into(), meta.clone());
                }

                crate::models::GraphEvent {
                    run_id: None,
                    thread_id: Some(batch.thread_id.clone()),
                    event_type: event_type.to_string(),
                    node_id: Some(evt.node_id.clone()),
                    actor: Some("oxidizedgraph".into()),
                    payload: Some(serde_json::Value::Object(payload)),
                }
            })
            .collect()
    }

    /// Adapt a checkpoint-save event into a fabric checkpoint request.
    pub fn adapt_to_checkpoint(
        batch: &GraphExecBatch,
        evt: &GraphExecEvent,
    ) -> Option<crate::models::CreateCheckpoint> {
        if evt.event_type != GraphExecEventType::CheckpointSave {
            return None;
        }
        let state = evt
            .state
            .clone()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        Some(crate::models::CreateCheckpoint {
            thread_id: batch.thread_id.clone(),
            node_id: evt.node_id.clone(),
            parent_id: evt.parent_node_id.clone(),
            state,
            metadata: evt.metadata.clone(),
        })
    }
}

// ── aivcs contract ──────────────────────────────────────────────

/// aivcs sends CI/CD pipeline lifecycle events via this contract.
/// Maps to: runs + artifacts + provenance events.
pub mod aivcs {
    use super::*;

    /// A pipeline run event from aivcs.
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
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

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
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

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    pub struct PipelineArtifact {
        pub key: String,
        pub content_type: Option<String>,
        pub size_bytes: Option<u64>,
        pub checksum: Option<String>,
    }

    /// Adapt a pipeline event into a fabric run creation request (for pipeline_start).
    pub fn adapt_to_run(evt: &PipelineEvent) -> Option<crate::models::CreateRun> {
        if evt.event_type != PipelineEventType::PipelineStart {
            return None;
        }
        let mut meta = serde_json::Map::new();
        meta.insert("source".into(), serde_json::Value::String("aivcs".into()));
        meta.insert(
            "pipeline_id".into(),
            serde_json::Value::String(evt.pipeline_id.clone()),
        );
        if let Some(ref sha) = evt.commit_sha {
            meta.insert("commit_sha".into(), serde_json::Value::String(sha.clone()));
        }
        if let Some(ref branch) = evt.branch {
            meta.insert("branch".into(), serde_json::Value::String(branch.clone()));
        }
        if let Some(ref user_meta) = evt.metadata {
            meta.insert("metadata".into(), user_meta.clone());
        }

        let trigger_suffix = match evt.event_type {
            PipelineEventType::PipelineStart => "pipeline_start",
            PipelineEventType::PipelineEnd => "pipeline_end",
            PipelineEventType::StageStart => "stage_start",
            PipelineEventType::StageEnd => "stage_end",
            PipelineEventType::TestResult => "test_result",
            PipelineEventType::BuildComplete => "build_complete",
            PipelineEventType::DeployStart => "deploy_start",
            PipelineEventType::DeployEnd => "deploy_end",
        };

        Some(crate::models::CreateRun {
            repo: evt.repo.clone(),
            trigger: Some(format!("aivcs:{trigger_suffix}")),
            actor: Some(evt.actor.clone()),
            metadata: Some(serde_json::Value::Object(meta)),
        })
    }

    /// Adapt a pipeline event into a fabric provenance event.
    pub fn adapt_to_event(evt: &PipelineEvent) -> crate::models::IngestEvent {
        let event_type = match evt.event_type {
            PipelineEventType::PipelineStart => "pipeline.start",
            PipelineEventType::PipelineEnd => "pipeline.end",
            PipelineEventType::StageStart => "stage.start",
            PipelineEventType::StageEnd => "stage.end",
            PipelineEventType::TestResult => "test.result",
            PipelineEventType::BuildComplete => "build.complete",
            PipelineEventType::DeployStart => "deploy.start",
            PipelineEventType::DeployEnd => "deploy.end",
        };

        let mut payload = serde_json::Map::new();
        payload.insert("source".into(), serde_json::Value::String("aivcs".into()));
        payload.insert(
            "pipeline_id".into(),
            serde_json::Value::String(evt.pipeline_id.clone()),
        );
        if let Some(ref sha) = evt.commit_sha {
            payload.insert("commit_sha".into(), serde_json::Value::String(sha.clone()));
        }
        if let Some(ref arts) = evt.artifacts {
            payload.insert(
                "artifacts".into(),
                serde_json::to_value(arts).unwrap_or_default(),
            );
        }
        if let Some(ref meta) = evt.metadata {
            payload.insert("metadata".into(), meta.clone());
        }

        crate::models::IngestEvent {
            run_id: evt.pipeline_id.clone(),
            event_type: event_type.to_string(),
            actor: evt.actor.clone(),
            entity_kind: None,
            entity_id: None,
            payload: Some(serde_json::Value::Object(payload)),
        }
    }
}

// ── llama.rs contract ───────────────────────────────────────────

/// llama.rs sends inference lifecycle events via this contract.
/// Maps to: MCP tasks + graph events (token/tool use telemetry).
pub mod llama_rs {
    use super::*;

    /// An inference request from llama.rs to be enqueued as a fabric task.
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
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

    /// A telemetry event from llama.rs inference execution.
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
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

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum InferenceTelemetryType {
        InferenceStart,
        InferenceEnd,
        ToolUse,
        TokenStream,
        InferenceError,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    pub struct InferenceToolCall {
        pub tool_name: String,
        pub input: serde_json::Value,
        pub output: Option<serde_json::Value>,
        pub duration_ms: Option<u64>,
    }

    /// Adapt an inference request into a fabric MCP task.
    pub fn adapt_to_task(req: &InferenceRequest) -> crate::models::CreateAgentTask {
        let mut params = serde_json::Map::new();
        params.insert(
            "source".into(),
            serde_json::Value::String("llama_rs".into()),
        );
        params.insert("model".into(), serde_json::Value::String(req.model.clone()));
        if let Some(ref prompt) = req.prompt {
            params.insert("prompt".into(), serde_json::Value::String(prompt.clone()));
        }
        if let Some(ref msgs) = req.messages {
            params.insert("messages".into(), msgs.clone());
        }
        if let Some(temp) = req.temperature {
            params.insert(
                "temperature".into(),
                serde_json::Value::Number(serde_json::Number::from_f64(temp).unwrap_or(0.into())),
            );
        }
        if let Some(max) = req.max_tokens {
            params.insert("max_tokens".into(), serde_json::Value::Number(max.into()));
        }
        if let Some(ref tools) = req.tools {
            params.insert(
                "tools".into(),
                serde_json::to_value(tools).unwrap_or_default(),
            );
        }
        if let Some(ref meta) = req.metadata {
            params.insert("metadata".into(), meta.clone());
        }

        crate::models::CreateAgentTask {
            job_id: req
                .run_id
                .clone()
                .unwrap_or_else(|| "llama_rs_inference".into()),
            task_type: "inference".into(),
            priority: 0,
            params: Some(serde_json::Value::Object(params)),
            graph_ref: None,
            play_id: None,
            parent_task_id: None,
            max_retries: Some(1),
        }
    }

    /// Adapt inference telemetry into fabric graph events.
    pub fn adapt_to_graph_events(telemetry: &InferenceTelemetry) -> Vec<crate::models::GraphEvent> {
        let mut events = Vec::new();

        let event_type = match telemetry.event_type {
            InferenceTelemetryType::InferenceStart => "inference.start",
            InferenceTelemetryType::InferenceEnd => "inference.end",
            InferenceTelemetryType::ToolUse => "inference.tool_use",
            InferenceTelemetryType::TokenStream => "inference.token_stream",
            InferenceTelemetryType::InferenceError => "inference.error",
        };

        let mut payload = serde_json::Map::new();
        payload.insert(
            "source".into(),
            serde_json::Value::String("llama_rs".into()),
        );
        payload.insert(
            "model".into(),
            serde_json::Value::String(telemetry.model.clone()),
        );
        if let Some(t_in) = telemetry.tokens_in {
            payload.insert("tokens_in".into(), serde_json::Value::Number(t_in.into()));
        }
        if let Some(t_out) = telemetry.tokens_out {
            payload.insert("tokens_out".into(), serde_json::Value::Number(t_out.into()));
        }
        if let Some(ms) = telemetry.duration_ms {
            payload.insert("duration_ms".into(), serde_json::Value::Number(ms.into()));
        }
        if let Some(ref err) = telemetry.error {
            payload.insert("error".into(), serde_json::Value::String(err.clone()));
        }
        if let Some(ref meta) = telemetry.metadata {
            payload.insert("metadata".into(), meta.clone());
        }

        events.push(crate::models::GraphEvent {
            run_id: None,
            thread_id: Some(telemetry.task_id.clone()),
            event_type: event_type.to_string(),
            node_id: None,
            actor: Some("llama_rs".into()),
            payload: Some(serde_json::Value::Object(payload)),
        });

        // Also emit individual tool-call events
        if let Some(ref tool_calls) = telemetry.tool_calls {
            for tc in tool_calls {
                let mut tc_payload = serde_json::Map::new();
                tc_payload.insert(
                    "source".into(),
                    serde_json::Value::String("llama_rs".into()),
                );
                tc_payload.insert(
                    "tool_name".into(),
                    serde_json::Value::String(tc.tool_name.clone()),
                );
                tc_payload.insert("input".into(), tc.input.clone());
                if let Some(ref output) = tc.output {
                    tc_payload.insert("output".into(), output.clone());
                }
                if let Some(ms) = tc.duration_ms {
                    tc_payload.insert("duration_ms".into(), serde_json::Value::Number(ms.into()));
                }

                events.push(crate::models::GraphEvent {
                    run_id: None,
                    thread_id: Some(telemetry.task_id.clone()),
                    event_type: "inference.tool_call".to_string(),
                    node_id: None,
                    actor: Some("llama_rs".into()),
                    payload: Some(serde_json::Value::Object(tc_payload)),
                });
            }
        }

        events
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── oxidizedgraph adapter tests ─────────────────────────

    #[test]
    fn oxidizedgraph_batch_serde_round_trip() {
        let batch = oxidizedgraph::GraphExecBatch {
            graph_id: "g1".into(),
            thread_id: "t1".into(),
            events: vec![oxidizedgraph::GraphExecEvent {
                event_type: oxidizedgraph::GraphExecEventType::NodeStart,
                node_id: "n1".into(),
                node_type: Some("llm_call".into()),
                parent_node_id: None,
                state: None,
                error: None,
                duration_ms: None,
                metadata: None,
            }],
        };
        let json = serde_json::to_string(&batch).unwrap();
        let parsed: oxidizedgraph::GraphExecBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(batch, parsed);
    }

    #[test]
    fn oxidizedgraph_adapt_to_graph_events() {
        let batch = oxidizedgraph::GraphExecBatch {
            graph_id: "g1".into(),
            thread_id: "t1".into(),
            events: vec![
                oxidizedgraph::GraphExecEvent {
                    event_type: oxidizedgraph::GraphExecEventType::NodeStart,
                    node_id: "n1".into(),
                    node_type: Some("tool".into()),
                    parent_node_id: None,
                    state: None,
                    error: None,
                    duration_ms: None,
                    metadata: None,
                },
                oxidizedgraph::GraphExecEvent {
                    event_type: oxidizedgraph::GraphExecEventType::NodeEnd,
                    node_id: "n1".into(),
                    node_type: None,
                    parent_node_id: None,
                    state: None,
                    error: None,
                    duration_ms: Some(150),
                    metadata: None,
                },
            ],
        };
        let events = oxidizedgraph::adapt_to_graph_events(&batch);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "node.start");
        assert_eq!(events[0].thread_id.as_deref(), Some("t1"));
        assert_eq!(events[0].actor.as_deref(), Some("oxidizedgraph"));
        assert_eq!(events[1].event_type, "node.end");
        let p = events[1].payload.as_ref().unwrap();
        assert_eq!(p["duration_ms"], 150);
        assert_eq!(p["source"], "oxidizedgraph");
    }

    #[test]
    fn oxidizedgraph_adapt_to_checkpoint() {
        let batch = oxidizedgraph::GraphExecBatch {
            graph_id: "g1".into(),
            thread_id: "t1".into(),
            events: vec![],
        };
        let evt = oxidizedgraph::GraphExecEvent {
            event_type: oxidizedgraph::GraphExecEventType::CheckpointSave,
            node_id: "n1".into(),
            node_type: None,
            parent_node_id: Some("n0".into()),
            state: Some(serde_json::json!({"messages": []})),
            error: None,
            duration_ms: None,
            metadata: None,
        };
        let cp = oxidizedgraph::adapt_to_checkpoint(&batch, &evt).unwrap();
        assert_eq!(cp.thread_id, "t1");
        assert_eq!(cp.node_id, "n1");
        assert_eq!(cp.parent_id.as_deref(), Some("n0"));
    }

    #[test]
    fn oxidizedgraph_non_checkpoint_returns_none() {
        let batch = oxidizedgraph::GraphExecBatch {
            graph_id: "g1".into(),
            thread_id: "t1".into(),
            events: vec![],
        };
        let evt = oxidizedgraph::GraphExecEvent {
            event_type: oxidizedgraph::GraphExecEventType::NodeStart,
            node_id: "n1".into(),
            node_type: None,
            parent_node_id: None,
            state: None,
            error: None,
            duration_ms: None,
            metadata: None,
        };
        assert!(oxidizedgraph::adapt_to_checkpoint(&batch, &evt).is_none());
    }

    // ── aivcs adapter tests ─────────────────────────────────

    #[test]
    fn aivcs_pipeline_event_serde_round_trip() {
        let evt = aivcs::PipelineEvent {
            pipeline_id: "p1".into(),
            repo: "stevedores-org/data-fabric".into(),
            event_type: aivcs::PipelineEventType::PipelineStart,
            commit_sha: Some("abc123".into()),
            branch: Some("main".into()),
            actor: "ci-bot".into(),
            artifacts: None,
            metadata: None,
        };
        let json = serde_json::to_string(&evt).unwrap();
        let parsed: aivcs::PipelineEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(evt, parsed);
    }

    #[test]
    fn aivcs_adapt_to_run() {
        let evt = aivcs::PipelineEvent {
            pipeline_id: "p1".into(),
            repo: "stevedores-org/data-fabric".into(),
            event_type: aivcs::PipelineEventType::PipelineStart,
            commit_sha: Some("abc123".into()),
            branch: Some("main".into()),
            actor: "ci-bot".into(),
            artifacts: None,
            metadata: None,
        };
        let run = aivcs::adapt_to_run(&evt).unwrap();
        assert_eq!(run.repo, "stevedores-org/data-fabric");
        assert_eq!(run.actor.as_deref(), Some("ci-bot"));
        assert_eq!(run.trigger.as_deref(), Some("aivcs:pipeline_start"));
        let meta = run.metadata.unwrap();
        assert_eq!(meta["source"], "aivcs");
        assert_eq!(meta["commit_sha"], "abc123");
    }

    #[test]
    fn aivcs_non_pipeline_start_returns_none() {
        let evt = aivcs::PipelineEvent {
            pipeline_id: "p1".into(),
            repo: "test".into(),
            event_type: aivcs::PipelineEventType::BuildComplete,
            commit_sha: None,
            branch: None,
            actor: "ci".into(),
            artifacts: None,
            metadata: None,
        };
        assert!(aivcs::adapt_to_run(&evt).is_none());
    }

    #[test]
    fn aivcs_adapt_to_event() {
        let evt = aivcs::PipelineEvent {
            pipeline_id: "p1".into(),
            repo: "test".into(),
            event_type: aivcs::PipelineEventType::BuildComplete,
            commit_sha: Some("def456".into()),
            branch: None,
            actor: "ci".into(),
            artifacts: Some(vec![aivcs::PipelineArtifact {
                key: "build/output.wasm".into(),
                content_type: Some("application/wasm".into()),
                size_bytes: Some(102400),
                checksum: Some("sha256:abc".into()),
            }]),
            metadata: None,
        };
        let fabric_evt = aivcs::adapt_to_event(&evt);
        assert_eq!(fabric_evt.event_type, "build.complete");
        assert_eq!(fabric_evt.run_id, "p1");
        assert_eq!(fabric_evt.actor, "ci");
        let p = fabric_evt.payload.unwrap();
        assert_eq!(p["source"], "aivcs");
    }

    // ── llama.rs adapter tests ──────────────────────────────

    #[test]
    fn llama_rs_inference_request_serde() {
        let req = llama_rs::InferenceRequest {
            model: "llama-3.2".into(),
            prompt: Some("hello".into()),
            messages: None,
            temperature: Some(0.7),
            max_tokens: Some(1024),
            tools: Some(vec!["web_search".into()]),
            run_id: Some("r1".into()),
            metadata: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: llama_rs::InferenceRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, parsed);
    }

    #[test]
    fn llama_rs_adapt_to_task() {
        let req = llama_rs::InferenceRequest {
            model: "llama-3.2".into(),
            prompt: Some("explain rust".into()),
            messages: None,
            temperature: Some(0.5),
            max_tokens: Some(512),
            tools: None,
            run_id: Some("r1".into()),
            metadata: None,
        };
        let task = llama_rs::adapt_to_task(&req);
        assert_eq!(task.task_type, "inference");
        assert_eq!(task.job_id, "r1");
        let params = task.params.unwrap();
        assert_eq!(params["model"], "llama-3.2");
        assert_eq!(params["source"], "llama_rs");
    }

    #[test]
    fn llama_rs_adapt_telemetry_to_events() {
        let telemetry = llama_rs::InferenceTelemetry {
            task_id: "t1".into(),
            event_type: llama_rs::InferenceTelemetryType::InferenceEnd,
            model: "llama-3.2".into(),
            tokens_in: Some(50),
            tokens_out: Some(200),
            duration_ms: Some(1500),
            tool_calls: Some(vec![llama_rs::InferenceToolCall {
                tool_name: "web_search".into(),
                input: serde_json::json!({"query": "rust"}),
                output: Some(serde_json::json!({"results": []})),
                duration_ms: Some(300),
            }]),
            error: None,
            metadata: None,
        };
        let events = llama_rs::adapt_to_graph_events(&telemetry);
        // 1 main event + 1 tool_call event
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "inference.end");
        assert_eq!(events[0].thread_id.as_deref(), Some("t1"));
        let p0 = events[0].payload.as_ref().unwrap();
        assert_eq!(p0["tokens_out"], 200);
        assert_eq!(events[1].event_type, "inference.tool_call");
        let p1 = events[1].payload.as_ref().unwrap();
        assert_eq!(p1["tool_name"], "web_search");
    }

    // ── Integration registry tests ──────────────────────────

    #[test]
    fn integration_target_serde() {
        let targets = vec![
            IntegrationTarget::Oxidizedgraph,
            IntegrationTarget::Aivcs,
            IntegrationTarget::LlamaRs,
        ];
        for target in &targets {
            let json = serde_json::to_string(target).unwrap();
            let parsed: IntegrationTarget = serde_json::from_str(&json).unwrap();
            assert_eq!(*target, parsed);
        }
    }

    #[test]
    fn register_integration_round_trip() {
        let reg = RegisterIntegration {
            target: IntegrationTarget::Oxidizedgraph,
            name: "my-graph-runner".into(),
            endpoint: Some("https://graph.example.com".into()),
            api_version: Some("v1".into()),
            config: None,
        };
        let json = serde_json::to_string(&reg).unwrap();
        let parsed: RegisterIntegration = serde_json::from_str(&json).unwrap();
        assert_eq!(reg, parsed);
    }
}
