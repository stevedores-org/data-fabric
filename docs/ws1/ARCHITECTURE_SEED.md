# WS1 Architecture Seed (Validated Against WS2)

This seed extracts stable service boundaries from `lornu.ai` and maps them onto the canonical WS2 entity model in this repository.

## 1) Extracted core service boundaries

### Boundary A: Control Plane API
- Source lineage: `crates/data-fabric/src/main.rs`, `handlers/fabric.rs`, `handlers/mcp.rs`.
- Responsibility:
  - receive orchestration commands (`play launch`, `task next`, `task response`),
  - expose fabric read/write API.
- Inputs: HTTP/MCP requests.
- Outputs: command acknowledgements, task payloads, memory payloads.

### Boundary B: Work Orchestration Engine
- Source lineage: `services/play_launcher.rs`, `services/task_queue.rs`, `shared-orchestrate-core`.
- Responsibility:
  - decompose play/plan into task units,
  - schedule, lease, retry, and complete tasks,
  - support human-gate/self-correcting loops.
- Inputs: play definitions, task state transitions, agent claims.
- Outputs: task assignments, status transitions, run outcomes.

### Boundary C: Policy and Data Safety
- Source lineage: `security/data_mesh.rs`, `security/pii_filter.rs`, `middleware/auth.rs`.
- Responsibility:
  - authn/authz over API and data layers,
  - PII detection and redaction policy enforcement,
  - layer/key scoped access control.
- Inputs: identity, request metadata, payload content.
- Outputs: allow/deny/escalate decisions and sanitized payloads.

### Boundary D: Persistence and Provenance
- Source lineage: `storage/*.rs`, `orchestrate-bridge`, `lornu-data` patterns.
- Responsibility:
  - durable writes for tasks/checkpoints/events/artifacts,
  - query surfaces for replay and lineage,
  - compatibility conversion for orchestration state.
- Inputs: entity write requests, checkpoint snapshots, graph events.
- Outputs: persisted canonical entities and queryable history.

### Boundary E: Federated Retrieval and Comms (optional acceleration plane)
- Source lineage: `apps/zero-copy-connector/src/*`, MCP hub dispatcher patterns.
- Responsibility:
  - federated query execution with pushdown,
  - streamed chunks for RAG/context assembly,
  - provider-dispatched comms/tools.
- Inputs: query intents, source configs, provider tool calls.
- Outputs: query results, stream chunks, tool invocation results.

## 2) Canonical dataflow (seed)

1. Client submits run/plan intent to Boundary A.
2. Boundary B materializes tasks, schedules assignments, and tracks retries.
3. Boundary C enforces policy and payload safety at each write boundary.
4. Boundary D records all state transitions and artifacts as canonical entities/events.
5. Boundary E enriches context through federated retrieval and returns chunks/tool results.

## 3) WS2 entity validation matrix

| WS2 entity | Boundary owner | Validation result |
|---|---|---|
| `Run` | A + B + D | Validated: run lifecycle naturally anchored in control-plane intake + orchestration transitions + provenance persistence |
| `Task` | B + D | Validated: queue/lease/retry model maps directly to canonical task state machine |
| `Plan` | B + D | Validated: play decomposition maps to plan graph semantics with task dependencies |
| `ToolCall` | E + D | Validated: MCP/provider dispatch and graph tool execution map to tool call records |
| `Artifact` | D (+ E for external retrieval outputs) | Validated: storage boundary supports typed artifact lineage |
| `PolicyDecision` | C + D | Validated: authz + PII checks yield explicit allow/deny/escalate decisions |
| `Release` | A + B + D | Validated: release is terminal aggregation of run/task/artifact outcomes |

Result: all required WS2 entities (`Run`, `Task`, `Plan`, `ToolCall`, `Artifact`, `PolicyDecision`, `Release`) are covered by extracted boundaries.

## 4) Seed implementation order (for follow-up WS)

1. Implement Boundary C parity first (policy + PII) to avoid unsafe growth.
2. Close Boundary B play decomposition and MCP contract parity.
3. Harden Boundary D replay/event lineage for observability and audit.
4. Add Boundary E federated retrieval as a velocity amplifier, not a blocker.
