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
}
