# WS1 Source Inventory Matrix

Baseline extraction source: `<lornu.ai-repo>/crates/data-fabric/src`.

## 1) Core crate module inventory (complete)

| Module | Purpose | Inputs | Outputs | Key dependencies |
|---|---|---|---|---|
| `lib.rs` | Library surface and module wiring for integration use | N/A | Re-exported modules | `config`, `models`, `services`, `storage`, `security` |
| `main.rs` | Service bootstrap and route composition | Env config, startup args | Running HTTP API | `axum`, `tower_http`, `state::AppState` |
| `config.rs` | Runtime configuration loading | Environment variables | Typed `Config` | `std::env` |
| `error.rs` | Unified error model and HTTP translation | Internal errors | JSON HTTP error responses | `axum`, `thiserror` |
| `state.rs` | Dependency graph assembly | `Config` | `AppState` with initialized services | `SurrealClient`, `TaskQueue`, `PlayLauncher`, `MemoryService`, `DataMesh` |
| `handlers/mod.rs` | Handler exports | N/A | Handler namespace | handler submodules |
| `handlers/health.rs` | Liveness/readiness endpoint logic | HTTP request | health payload | `axum` |
| `handlers/fabric.rs` | Play lifecycle endpoints (`launch/list/status`) | `LaunchPlayRequest`, path/query params | `LaunchPlayResponse`, play status payload | `PlayLauncher`, `AppState` |
| `handlers/mcp.rs` | Task polling, task result, memory I/O, queue stats endpoints | MCP task/memory requests | task assignment/results, memory responses | `TaskQueue`, `MemoryService` |
| `services/mod.rs` | Service exports | N/A | service namespace | service submodules |
| `services/play_launcher.rs` | Converts play definitions into queued tasks and tracks play state | `play_name`, params | persisted play + task set | `TaskQueue`, `models::Play` |
| `services/task_queue.rs` | Priority queue + persistence + timeout sweeper and retries | `McpTask`, agent id, task responses | assigned/completed/failed task state | `DashMap`, `BinaryHeap`, `SurrealClient` |
| `services/memory.rs` | Bronze/Silver/Gold memory workflow with PII + mesh policy | `SaveMemoryRequest`, read parameters | `MemoryResponse`, filtered/promoted entries | `BronzeStorage`, `SilverStorage`, `GoldStorage`, `PiiFilter`, `DataMesh` |
| `storage/mod.rs` | Storage exports/utilities (`hash`, URL redaction) | N/A | storage namespace | storage submodules |
| `storage/surreal.rs` | DB client wrapper and connection logic | Surreal URL/ns/db credentials | connected `SurrealClient` | `surrealdb` |
| `storage/migrations.rs` | Applies schema/index migrations | DB client | migrated schema state | `SurrealClient` |
| `storage/bronze.rs` | Raw memory persistence | `MemoryEntry` | `memory_bronze` records | `SurrealClient`, content hashing |
| `storage/silver.rs` | PII-filtered memory persistence | filtered `MemoryEntry` | `memory_silver` records | `SurrealClient`, content hashing |
| `storage/gold.rs` | Curated memory persistence + tag search/stats | curated `MemoryEntry`, tag filters | `memory_gold` records + aggregate stats | `SurrealClient` |
| `models/mod.rs` | Model exports | N/A | model namespace | model submodules |
| `models/task.rs` | MCP task schema and task lifecycle methods | task creation + status transitions | serialized task payloads | `chrono`, `serde`, `uuid` |
| `models/memory.rs` | Bronze/Silver/Gold memory schema and requests | memory write/read payloads | `MemoryEntry`, `MemoryResponse` | `serde_json`, `chrono` |
| `models/play.rs` | Play definition + runtime instance schema | play requests/params | play/task relationship state | `serde_json`, `chrono`, `uuid` |
| `models/knowledge.rs` | Knowledge/metrics domain models | domain-specific payloads | typed knowledge structs | `serde` |
| `security/mod.rs` | Security exports | N/A | security namespace | security submodules |
| `security/pii_filter.rs` | PII detection/redaction for JSON payloads | arbitrary JSON value | findings + redacted value | `regex`, JSON tree walk |
| `security/data_mesh.rs` | Layer/key-level access policy model with persistence | agent id, action, key/layer | allow/deny decisions | `RwLock`, persisted `agent_permissions` |
| `middleware/mod.rs` | Middleware exports | N/A | middleware namespace | middleware submodules |
| `middleware/auth.rs` | Bearer token auth + constant-time compare | HTTP Authorization header | allow/deny response | `axum`, constant-time check |

## 2) Extracted capability/dataflow map

### API and orchestration flow
1. `handlers/fabric.rs` receives play launch request.
2. `services/play_launcher.rs` expands play into task graph fragments.
3. `services/task_queue.rs` persists and prioritizes tasks.
4. `handlers/mcp.rs` serves `GET /mcp/task/next` for agent pull.
5. agent result on `POST /mcp/response` updates task + play status.

### Memory/data-layer flow
1. `POST /mcp/memory` enters `services/memory.rs`.
2. `security/data_mesh.rs` enforces write authorization by layer/key.
3. `security/pii_filter.rs` scans/filters based on target layer.
4. `storage/{bronze|silver|gold}.rs` persists by policy tier.
5. reads default to Gold -> Silver -> Bronze fallback order.

### Security/control flow
- `middleware/auth.rs` protects all non-public routes.
- `DataMesh` controls per-agent layer and key-prefix access.
- `PiiFilter` blocks Gold on PII and sanitizes Silver.

## 3) Non-primary but relevant extraction sources

| Source area | Reusable pattern extracted | Why it matters to WS1 seed |
|---|---|---|
| `lornu.ai/crates/lornu-data/src` | Standardized SurrealDB init/retry/config + `Repository<T>` trait | Base data-access pattern to port to D1-compatible repository interfaces |
| `lornu.ai/apps/zero-copy-connector/src` | Federated query model, predicate pushdown, streaming chunks | Shapes future fabric-comms + RAG ingestion/query interfaces |
| `lornu.ai/crates/shared-orchestrate-core/src` | Graph execution state machine, retry/self-correction/HITL model | Directly informs Run/Task/Plan/ToolCall lifecycle semantics |
| `lornu.ai/crates/orchestrate-bridge/src` | Checkpoint and event conversion between orchestration runtimes | Useful for compatibility adapters and provenance continuity |
| `lornu.ai/packages/lornu-mcp-hub-rs/src` | MCP resource/tool dispatch and provider registry | Defines orchestration-facing comms contract over MCP |
| `lornu.ai/ai-agents/ai-agent-orchestrator/src` | Task delegation + graph-run fallback orchestration | Helps define autonomous code-development task loop boundaries |

## 4) Current `data-fabric` repo status vs extracted modules

- Already present in this repo (`src/models/*`, `src/db.rs`, migrations):
  - WS2 canonical entity model,
  - M1 task queue + agent registration primitives,
  - M2 checkpoint persistence wiring,
  - M3 graph event ingress skeleton.
- Not yet present from extracted baseline:
  - policy-grade mesh + PII controls,
  - full play-launch/decomposition service,
  - federated zero-copy query path,
  - orchestrator bridge compatibility layer.
