# WS1 Capability Map and Gap Analysis

Scoring:
- `impact` (1-5): effect on autonomous agent-builder velocity.
- `urgency` (1-5): near-term dependency risk for WS3-WS6.
- `priority_score = impact * urgency`.

## 1) Capability map (extracted vs target)

| Capability | Exists in `lornu.ai` baseline | Exists in this repo now | Gap | impact | urgency | priority_score |
|---|---|---|---|---:|---:|---:|
| Run/Task/Plan/ToolCall/Artifact/PolicyDecision/Release canonical schema | Partial (different domain model) | Yes (WS2 entities) | None (monitor drift) | 5 | 3 | 15 |
| Agent task queue with retry/lease semantics | Yes (`services/task_queue.rs`) | Yes (`src/db.rs` tasks + retries/lease) | Minor parity validation | 4 | 3 | 12 |
| MCP-style task poll + response loop | Yes (`handlers/mcp.rs`) | Partial (M1 API subset) | Endpoint and contract hardening | 4 | 4 | 16 |
| Play launch/decomposition engine | Yes (`services/play_launcher.rs`) | No | Missing orchestration entrypoint | 5 | 4 | 20 |
| Data layer controls (Bronze/Silver/Gold + promotion paths) | Yes | Partial (entity-level only) | Missing tiered memory layer semantics | 4 | 4 | 16 |
| PII filtering with structured redaction | Yes (`security/pii_filter.rs`) | No | Missing compliance guardrail | 5 | 5 | 25 |
| Data mesh policy model (layer/key access) | Yes (`security/data_mesh.rs`) | Partial (policy decisions entity exists, no enforcement engine) | Missing runtime authorization fabric | 5 | 5 | 25 |
| Checkpoint persistence for graph execution | Partial via bridge + orchestration | Yes (M2 checkpoint model + APIs) | Storage/perf hardening | 4 | 3 | 12 |
| Orchestration event pipeline | Yes (bridge events, orchestrator telemetry) | Partial (M3 graph event ingest) | Missing downstream processing/consumer path | 4 | 4 | 16 |
| Zero-copy federated query + pushdown | Yes (`zero-copy-connector`) | No | Missing high-velocity cross-source retrieval | 4 | 3 | 12 |
| Streaming query/chunk interface for RAG | Yes (`query/streaming.py`) | No | Missing low-latency context feed | 4 | 3 | 12 |
| Orchestrate bridge compatibility adapters | Yes (`crates/orchestrate-bridge`) | No | Missing interoperability layer | 3 | 3 | 9 |

## 2) Priority gaps to schedule next

### P0 (build now)
1. `P0-1` policy enforcement runtime (`DataMesh`-like authorization) - score `25`.
2. `P0-2` PII filtering and redaction path for non-bronze data - score `25`.
3. `P0-3` play launch/decomposition endpoint into task graph - score `20`.

### P1 (immediately after P0)
1. MCP contract hardening + event-pipeline consumption - score `16`.
2. Bronze/Silver/Gold lifecycle parity (promotion and query semantics) - score `16`.
3. Graph event processing pipeline (queueing and replayability) - score `16`.

### P2 (performance amplification)
1. Zero-copy connector + predicate pushdown integration - score `12`.
2. Streamed retrieval interface for RAG context assembly - score `12`.
3. Orchestrator bridge adapters for runtime compatibility - score `9`.

## 3) Explicit acceptance-criteria check

- Requirement: identify at least 3 missing capabilities.
- Identified missing capabilities (>=3):
  - Play launch/decomposition engine.
  - PII filtering/redaction service.
  - Data mesh authorization enforcement.
  - Zero-copy federated query path.
  - Streaming RAG query interface.
  - Orchestrate bridge compatibility layer.
