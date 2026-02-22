use serde::{Deserialize, Serialize};

// ── Runs (WS2: core domain entity) ─────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateRun {
    pub repo: String,
    pub trigger: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RunCreated {
    pub id: String,
    pub status: String,
    pub repo: String,
}

// ── Provenance Events (WS3: append-only audit trail) ────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ProvenanceEvent {
    pub run_id: String,
    pub event_type: String,
    pub actor: String,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EventAck {
    pub id: String,
    pub event_type: String,
    pub accepted: bool,
}

// ── Policy (WS4: governance layer) ──────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyCheckRequest {
    pub action: String,
    pub actor: String,
    pub resource: Option<String>,
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyCheckResponse {
    pub action: String,
    pub decision: String,
    pub reason: String,
}

// ── Tasks (M1: agent task queue) ────────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct CreateTask {
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
pub struct Task {
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

// ── Agents (M1: agent registration) ─────────────────────────────

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

// ── Common ──────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_run_round_trip() {
        let input =
            r#"{"repo":"stevedores-org/data-fabric","trigger":"push","metadata":{"ref":"main"}}"#;
        let parsed: CreateRun = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.repo, "stevedores-org/data-fabric");
        assert_eq!(parsed.trigger.as_deref(), Some("push"));
        let json = serde_json::to_string(&parsed).unwrap();
        let reparsed: CreateRun = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn create_run_minimal() {
        let input = r#"{"repo":"my-repo"}"#;
        let parsed: CreateRun = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.repo, "my-repo");
        assert!(parsed.trigger.is_none());
        assert!(parsed.metadata.is_none());
    }

    #[test]
    fn run_created_serializes() {
        let run = RunCreated {
            id: "abc123".into(),
            status: "created".into(),
            repo: "test-repo".into(),
        };
        let json = serde_json::to_value(&run).unwrap();
        assert_eq!(json["id"], "abc123");
        assert_eq!(json["status"], "created");
        assert_eq!(json["repo"], "test-repo");
    }

    #[test]
    fn provenance_event_round_trip() {
        let input =
            r#"{"run_id":"r1","event_type":"build.start","actor":"ci-bot","payload":{"step":1}}"#;
        let parsed: ProvenanceEvent = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.event_type, "build.start");
        let json = serde_json::to_string(&parsed).unwrap();
        let reparsed: ProvenanceEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn event_ack_serializes() {
        let ack = EventAck {
            id: "evt1".into(),
            event_type: "build.start".into(),
            accepted: true,
        };
        let json = serde_json::to_value(&ack).unwrap();
        assert_eq!(json["accepted"], true);
    }

    #[test]
    fn policy_check_round_trip() {
        let input =
            r#"{"action":"deploy","actor":"dev@example.com","resource":"prod","context":null}"#;
        let parsed: PolicyCheckRequest = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.action, "deploy");
        assert_eq!(parsed.resource.as_deref(), Some("prod"));
        let json = serde_json::to_string(&parsed).unwrap();
        let reparsed: PolicyCheckRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn policy_response_serializes() {
        let resp = PolicyCheckResponse {
            action: "deploy".into(),
            decision: "allow".into(),
            reason: "no restrictions".into(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["decision"], "allow");
    }

    #[test]
    fn create_task_round_trip() {
        let input = r#"{"job_id":"j1","task_type":"build","priority":5,"params":{"repo":"test"}}"#;
        let parsed: CreateTask = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.job_id, "j1");
        assert_eq!(parsed.task_type, "build");
        assert_eq!(parsed.priority, 5);
        let json = serde_json::to_string(&parsed).unwrap();
        let reparsed: CreateTask = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn create_task_minimal() {
        let input = r#"{"job_id":"j1","task_type":"build"}"#;
        let parsed: CreateTask = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.priority, 0);
        assert!(parsed.params.is_none());
        assert!(parsed.graph_ref.is_none());
    }

    #[test]
    fn register_agent_round_trip() {
        let input =
            r#"{"name":"build-agent","capabilities":["build","test"],"endpoint":"https://agent.example.com"}"#;
        let parsed: RegisterAgent = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.name, "build-agent");
        assert_eq!(parsed.capabilities, vec!["build", "test"]);
        assert_eq!(
            parsed.endpoint.as_deref(),
            Some("https://agent.example.com")
        );
    }

    #[test]
    fn create_checkpoint_round_trip() {
        let input = r#"{"thread_id":"t1","node_id":"n1","state":{"messages":[]}}"#;
        let parsed: CreateCheckpoint = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.thread_id, "t1");
        assert_eq!(parsed.node_id, "n1");
        assert!(parsed.parent_id.is_none());
    }

    #[test]
    fn graph_event_batch_round_trip() {
        let input = r#"{"events":[{"event_type":"node.start","node_id":"n1","thread_id":"t1"}]}"#;
        let parsed: GraphEventBatch = serde_json::from_str(input).unwrap();
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].event_type, "node.start");
    }
}
