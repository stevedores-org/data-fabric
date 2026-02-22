use super::*;

// ── Entity round-trips ──────────────────────────────────────────

#[test]
fn run_round_trip() {
    let run = Run {
        id: "r1".into(),
        repo: "stevedores-org/data-fabric".into(),
        status: Status::Running,
        trigger: Some("push".into()),
        actor: "ci-bot".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:01Z".into(),
        metadata: None,
    };
    let json = serde_json::to_string(&run).unwrap();
    let parsed: Run = serde_json::from_str(&json).unwrap();
    assert_eq!(run, parsed);
}

#[test]
fn task_round_trip() {
    let task = Task {
        id: "t1".into(),
        run_id: "r1".into(),
        plan_id: Some("p1".into()),
        name: "run clippy".into(),
        status: Status::Created,
        actor: Some("agent-1".into()),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        metadata: None,
    };
    let json = serde_json::to_string(&task).unwrap();
    let parsed: Task = serde_json::from_str(&json).unwrap();
    assert_eq!(task, parsed);
}

#[test]
fn plan_round_trip() {
    let plan = Plan {
        id: "p1".into(),
        run_id: "r1".into(),
        name: "ci pipeline".into(),
        status: Status::Created,
        task_ids: vec!["t1".into(), "t2".into()],
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        metadata: None,
    };
    let json = serde_json::to_string(&plan).unwrap();
    let parsed: Plan = serde_json::from_str(&json).unwrap();
    assert_eq!(plan, parsed);
    assert_eq!(parsed.task_ids.len(), 2);
}

#[test]
fn tool_call_round_trip() {
    let tc = ToolCall {
        id: "tc1".into(),
        run_id: "r1".into(),
        task_id: Some("t1".into()),
        tool_name: "cargo_clippy".into(),
        input: serde_json::json!({"workspace": true}),
        output: Some(serde_json::json!({"warnings": 0})),
        status: Status::Succeeded,
        duration_ms: Some(1200),
        created_at: "2026-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&tc).unwrap();
    let parsed: ToolCall = serde_json::from_str(&json).unwrap();
    assert_eq!(tc, parsed);
}

#[test]
fn artifact_round_trip() {
    let art = Artifact {
        id: "a1".into(),
        run_id: "r1".into(),
        key: "build/output.wasm".into(),
        content_type: Some("application/wasm".into()),
        size_bytes: 102400,
        checksum: "sha256:abc123".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        metadata: None,
    };
    let json = serde_json::to_string(&art).unwrap();
    let parsed: Artifact = serde_json::from_str(&json).unwrap();
    assert_eq!(art, parsed);
}

#[test]
fn policy_decision_round_trip() {
    let pd = PolicyDecision {
        id: "pd1".into(),
        run_id: Some("r1".into()),
        action: "deploy".into(),
        actor: "agent-1".into(),
        resource: Some("prod".into()),
        decision: PolicyVerdict::Allow,
        reason: "within risk threshold".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        context: None,
    };
    let json = serde_json::to_string(&pd).unwrap();
    let parsed: PolicyDecision = serde_json::from_str(&json).unwrap();
    assert_eq!(pd, parsed);
}

#[test]
fn release_round_trip() {
    let rel = Release {
        id: "rel1".into(),
        repo: "stevedores-org/data-fabric".into(),
        version: "0.2.0".into(),
        run_id: "r1".into(),
        artifact_ids: vec!["a1".into(), "a2".into()],
        status: Status::Succeeded,
        created_at: "2026-01-01T00:00:00Z".into(),
        metadata: None,
    };
    let json = serde_json::to_string(&rel).unwrap();
    let parsed: Release = serde_json::from_str(&json).unwrap();
    assert_eq!(rel, parsed);
}

#[test]
fn event_round_trip() {
    let evt = Event {
        id: "e1".into(),
        run_id: "r1".into(),
        entity_kind: EntityKind::Task,
        entity_id: "t1".into(),
        event_type: "task.started".into(),
        actor: "agent-1".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        payload: Some(serde_json::json!({"attempt": 1})),
    };
    let json = serde_json::to_string(&evt).unwrap();
    let parsed: Event = serde_json::from_str(&json).unwrap();
    assert_eq!(evt, parsed);
}

// ── Status serde ────────────────────────────────────────────────

#[test]
fn status_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&Status::Created).unwrap(),
        r#""created""#
    );
    assert_eq!(
        serde_json::to_string(&Status::Running).unwrap(),
        r#""running""#
    );
    assert_eq!(
        serde_json::to_string(&Status::Succeeded).unwrap(),
        r#""succeeded""#
    );
    assert_eq!(
        serde_json::to_string(&Status::Failed).unwrap(),
        r#""failed""#
    );
    assert_eq!(
        serde_json::to_string(&Status::Cancelled).unwrap(),
        r#""cancelled""#
    );
}

#[test]
fn policy_verdict_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&PolicyVerdict::Allow).unwrap(),
        r#""allow""#
    );
    assert_eq!(
        serde_json::to_string(&PolicyVerdict::Deny).unwrap(),
        r#""deny""#
    );
    assert_eq!(
        serde_json::to_string(&PolicyVerdict::Escalate).unwrap(),
        r#""escalate""#
    );
}

// ── Relationship serde ──────────────────────────────────────────

#[test]
fn causality_round_trip() {
    let c = Relationship::Causality(Causality {
        from_kind: EntityKind::Run,
        from_id: "r1".into(),
        to_kind: EntityKind::Task,
        to_id: "t1".into(),
        relation: "spawned".into(),
    });
    let json = serde_json::to_string(&c).unwrap();
    assert!(json.contains(r#""type":"causality""#));
    let parsed: Relationship = serde_json::from_str(&json).unwrap();
    assert_eq!(c, parsed);
}

#[test]
fn dependency_round_trip() {
    let d = Relationship::Dependency(Dependency {
        source_kind: EntityKind::Task,
        source_id: "t2".into(),
        depends_on_kind: EntityKind::Task,
        depends_on_id: "t1".into(),
    });
    let json = serde_json::to_string(&d).unwrap();
    let parsed: Relationship = serde_json::from_str(&json).unwrap();
    assert_eq!(d, parsed);
}

#[test]
fn ownership_round_trip() {
    let o = Relationship::Ownership(Ownership {
        entity_kind: EntityKind::Artifact,
        entity_id: "a1".into(),
        owner: "agent-1".into(),
    });
    let json = serde_json::to_string(&o).unwrap();
    let parsed: Relationship = serde_json::from_str(&json).unwrap();
    assert_eq!(o, parsed);
}

#[test]
fn lineage_round_trip() {
    let l = Relationship::Lineage(Lineage {
        entity_kind: EntityKind::Artifact,
        entity_id: "a2".into(),
        parent_kind: EntityKind::Artifact,
        parent_id: "a1".into(),
    });
    let json = serde_json::to_string(&l).unwrap();
    let parsed: Relationship = serde_json::from_str(&json).unwrap();
    assert_eq!(l, parsed);
}

// ── Request types ───────────────────────────────────────────────

#[test]
fn create_run_request() {
    let input =
        r#"{"repo":"stevedores-org/data-fabric","trigger":"push","metadata":{"ref":"main"}}"#;
    let parsed: CreateRun = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.repo, "stevedores-org/data-fabric");
    assert_eq!(parsed.trigger.as_deref(), Some("push"));
}

#[test]
fn create_run_minimal() {
    let input = r#"{"repo":"my-repo"}"#;
    let parsed: CreateRun = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.repo, "my-repo");
    assert!(parsed.trigger.is_none());
    assert!(parsed.actor.is_none());
}

#[test]
fn create_task_request() {
    let input = r#"{"run_id":"r1","name":"clippy","actor":"agent-1"}"#;
    let parsed: CreateTask = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.run_id, "r1");
    assert_eq!(parsed.name, "clippy");
}

#[test]
fn create_plan_request() {
    let input = r#"{"run_id":"r1","name":"ci","task_ids":["t1","t2"]}"#;
    let parsed: CreatePlan = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.task_ids.as_ref().unwrap().len(), 2);
}

#[test]
fn record_tool_call_request() {
    let input = r#"{"run_id":"r1","tool_name":"cargo_test","input":{"workspace":true}}"#;
    let parsed: RecordToolCall = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.tool_name, "cargo_test");
    assert!(parsed.output.is_none());
}

#[test]
fn ingest_event_request() {
    let input = r#"{"run_id":"r1","event_type":"build.start","actor":"ci-bot"}"#;
    let parsed: IngestEvent = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.event_type, "build.start");
}

#[test]
fn policy_check_request() {
    let input = r#"{"action":"deploy","actor":"dev@example.com","resource":"prod"}"#;
    let parsed: PolicyCheckRequest = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.action, "deploy");
}

#[test]
fn create_release_request() {
    let input = r#"{"repo":"stevedores-org/data-fabric","version":"0.2.0","run_id":"r1","artifact_ids":["a1"]}"#;
    let parsed: CreateRelease = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.version, "0.2.0");
}

#[test]
fn created_response() {
    let resp = Created {
        id: "abc123".into(),
        status: "created".into(),
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["id"], "abc123");
}

#[test]
fn event_ack_response() {
    let ack = EventAck {
        id: "evt1".into(),
        event_type: "build.start".into(),
        accepted: true,
    };
    let json = serde_json::to_value(&ack).unwrap();
    assert_eq!(json["accepted"], true);
}

#[test]
fn policy_check_response() {
    let resp = PolicyCheckResponse {
        id: "pd1".into(),
        action: "deploy".into(),
        decision: "allow".into(),
        reason: "no restrictions".into(),
        risk_level: "write".into(),
        matched_rule: Some("rule1".into()),
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["decision"], "allow");
    assert_eq!(json["risk_level"], "write");
    assert_eq!(json["matched_rule"], "rule1");
}

// ── WS4 Policy Rules ────────────────────────────────────────────

#[test]
fn create_policy_rule_defaults() {
    let input = r#"{"name":"test","verdict":"allow"}"#;
    let parsed: CreatePolicyRule = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.action_pattern, "*");
    assert_eq!(parsed.resource_pattern, "*");
    assert_eq!(parsed.actor_pattern, "*");
    assert_eq!(parsed.risk_level, "read");
    assert_eq!(parsed.priority, 0);
}

#[test]
fn create_policy_rule_full() {
    let input = r#"{
        "name": "block prod deploys",
        "action_pattern": "deploy",
        "resource_pattern": "prod",
        "actor_pattern": "*",
        "risk_level": "irreversible",
        "verdict": "deny",
        "reason": "prod deploys require approval",
        "priority": 100
    }"#;
    let parsed: CreatePolicyRule = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.name, "block prod deploys");
    assert_eq!(parsed.verdict, "deny");
    assert_eq!(parsed.priority, 100);
}

#[test]
fn update_policy_rule_partial() {
    let input = r#"{"verdict":"deny","enabled":false}"#;
    let parsed: UpdatePolicyRule = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.verdict.as_deref(), Some("deny"));
    assert_eq!(parsed.enabled, Some(false));
    assert!(parsed.name.is_none());
    assert!(parsed.action_pattern.is_none());
}

#[test]
fn policy_rule_response_round_trip() {
    let resp = PolicyRuleResponse {
        id: "r1".into(),
        name: "test rule".into(),
        action_pattern: "deploy:*".into(),
        resource_pattern: "prod".into(),
        actor_pattern: "*".into(),
        risk_level: "destructive".into(),
        verdict: "escalate".into(),
        reason: "needs approval".into(),
        priority: 10,
        enabled: true,
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let parsed: PolicyRuleResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, parsed);
}

#[test]
fn policy_decision_response_round_trip() {
    let resp = PolicyDecisionResponse {
        id: "d1".into(),
        action: "deploy".into(),
        actor: "agent-1".into(),
        resource: Some("staging".into()),
        decision: "allow".into(),
        reason: "matched rule".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        context: Some(serde_json::json!({"env": "staging"})),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let parsed: PolicyDecisionResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, parsed);
}

// ── Orchestration types (M1-M3) ────────────────────────────────

#[test]
fn create_agent_task_round_trip() {
    let input = r#"{"job_id":"j1","task_type":"build","priority":5,"params":{"repo":"test"}}"#;
    let parsed: CreateAgentTask = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.job_id, "j1");
    assert_eq!(parsed.task_type, "build");
    assert_eq!(parsed.priority, 5);
    let json = serde_json::to_string(&parsed).unwrap();
    let reparsed: CreateAgentTask = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, reparsed);
}

#[test]
fn create_agent_task_minimal() {
    let input = r#"{"job_id":"j1","task_type":"build"}"#;
    let parsed: CreateAgentTask = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.priority, 0);
    assert!(parsed.params.is_none());
    assert!(parsed.graph_ref.is_none());
}

#[test]
fn agent_task_created_serializes() {
    let tc = TaskCreated {
        id: "abc".into(),
        status: "pending".into(),
    };
    let json = serde_json::to_value(&tc).unwrap();
    assert_eq!(json["status"], "pending");
}

#[test]
fn task_complete_request_round_trip() {
    let input = r#"{"result":{"output":"done"}}"#;
    let parsed: TaskCompleteRequest = serde_json::from_str(input).unwrap();
    assert!(parsed.result.is_some());
}

#[test]
fn task_fail_request_round_trip() {
    let input = r#"{"error":"timeout"}"#;
    let parsed: TaskFailRequest = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.error, "timeout");
}

// ── M1: Agents ──────────────────────────────────────────────────

#[test]
fn register_agent_round_trip() {
    let input = r#"{"name":"build-agent","capabilities":["build","test"],"endpoint":"https://agent.example.com"}"#;
    let parsed: RegisterAgent = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.name, "build-agent");
    assert_eq!(parsed.capabilities, vec!["build", "test"]);
    assert_eq!(
        parsed.endpoint.as_deref(),
        Some("https://agent.example.com")
    );
}

// ── M2: Checkpoints ────────────────────────────────────────────

#[test]
fn create_checkpoint_round_trip() {
    let input = r#"{"thread_id":"t1","node_id":"n1","state":{"messages":[]}}"#;
    let parsed: CreateCheckpoint = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.thread_id, "t1");
    assert_eq!(parsed.node_id, "n1");
    assert!(parsed.parent_id.is_none());
}

#[test]
fn checkpoint_created_serializes() {
    let cc = CheckpointCreated {
        id: "cp1".into(),
        thread_id: "t1".into(),
        state_r2_key: "checkpoints/t1/cp1".into(),
    };
    let json = serde_json::to_value(&cc).unwrap();
    assert_eq!(json["thread_id"], "t1");
}

// ── M3: Graph Events ───────────────────────────────────────────

#[test]
fn graph_event_batch_round_trip() {
    let input = r#"{"events":[{"event_type":"node.start","node_id":"n1","thread_id":"t1"}]}"#;
    let parsed: GraphEventBatch = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.events.len(), 1);
    assert_eq!(parsed.events[0].event_type, "node.start");
}

#[test]
fn graph_event_ack_serializes() {
    let ack = GraphEventAck {
        accepted: 5,
        queued: true,
    };
    let json = serde_json::to_value(&ack).unwrap();
    assert_eq!(json["accepted"], 5);
    assert_eq!(json["queued"], true);
}

#[test]
fn error_response_serializes() {
    let err = ErrorResponse {
        error: "something went wrong".into(),
    };
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json["error"], "something went wrong");
}

// ── WS3 Trace / Provenance (#43) ──────────────────────────────

#[test]
fn trace_event_round_trip() {
    let evt = TraceEvent {
        id: "ev1".into(),
        run_id: Some("r1".into()),
        thread_id: Some("t1".into()),
        event_type: "node.start".into(),
        node_id: Some("n1".into()),
        actor: Some("agent-1".into()),
        payload: Some(serde_json::json!({"step": 1})),
        created_at: "2026-02-22T12:00:00Z".into(),
    };
    let json = serde_json::to_value(&evt).unwrap();
    assert_eq!(json["event_type"], "node.start");
    let parsed: TraceEvent = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.id, evt.id);
}

#[test]
fn trace_response_serializes() {
    let resp = TraceResponse {
        run_id: "r1".into(),
        events: vec![TraceEvent {
            id: "ev1".into(),
            run_id: None,
            thread_id: None,
            event_type: "run.start".into(),
            node_id: None,
            actor: None,
            payload: None,
            created_at: "2026-02-22T12:00:00Z".into(),
        }],
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["run_id"], "r1");
    assert_eq!(json["events"].as_array().unwrap().len(), 1);
}

// ── WS5: Retrieval & Memory ────────────────────────────────────

#[test]
fn upsert_memory_item_round_trip() {
    let input = r#"{
        "repo":"stevedores-org/data-fabric",
        "kind":"checkpoint",
        "run_id":"r1",
        "thread_id":"th-1",
        "summary":"checkpoint after codegen",
        "tags":["ci","checkpoint"],
        "success_rate":0.9,
        "ttl_seconds":3600
    }"#;
    let parsed: UpsertMemoryItemRequest = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.repo, "stevedores-org/data-fabric");
    assert_eq!(parsed.kind, MemoryKind::Checkpoint);
    assert_eq!(parsed.tags.len(), 2);
}

#[test]
fn retrieve_memory_defaults_apply() {
    let input = r#"{"repo":"stevedores-org/data-fabric","query":"fix failing checks"}"#;
    let parsed: RetrieveMemoryRequest = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.top_k, 8);
    assert!(!parsed.include_stale);
    assert!(!parsed.include_unsafe);
    assert!(!parsed.include_conflicted);
}

#[test]
fn context_pack_request_defaults_budget() {
    let input =
        r#"{"repo":"stevedores-org/data-fabric","query":"orchestrator retry loop","top_k":5}"#;
    let parsed: ContextPackRequest = serde_json::from_str(input).unwrap();
    assert_eq!(parsed.token_budget, 4096);
    assert_eq!(parsed.retrieval.top_k, 5);
}

#[test]
fn retrieval_feedback_round_trip() {
    let input = r#"{
        "query_id":"q1",
        "success":true,
        "first_pass_success":false,
        "cache_hit":true,
        "latency_ms":120
    }"#;
    let parsed: RetrievalFeedback = serde_json::from_str(input).unwrap();
    assert!(parsed.success);
    assert!(parsed.cache_hit);
    assert_eq!(parsed.latency_ms, Some(120));
}
