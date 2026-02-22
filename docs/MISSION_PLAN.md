# Mission Plan

## Phase WS1: Source Extraction Baseline (Issue #42)
- source inventory matrix completed: `docs/ws1/SOURCE_INVENTORY_MATRIX.md`
- capability gap map completed: `docs/ws1/CAPABILITY_GAP_MAP.md`
- migration risk register completed: `docs/ws1/MIGRATION_RISK_REGISTER.md`
- architecture seed completed: `docs/ws1/ARCHITECTURE_SEED.md`

## Phase M0: Contracts and Skeleton
- finalize canonical entities: task, memory, run_event, artifact, context_pack, policy_decision
- lock API contracts for ingest/query/task-loop
- stand up worker skeleton + health

## Phase M1: Core Fabric IO
- ingest pipeline (bronze)
- normalization path (silver)
- task loop (claim/ack) over MCP endpoints

## Phase M2: RAG Acceleration
- vector index and retrieval endpoint
- context pack assembly and KV caching
- confidence-based fallback policy

## Phase M3: Orchestration Adapters
- adapter spec for oxidizedgraph
- provenance bridge for aivcs
- deep retrieval adapter to oxidizedRAG

## Phase M4: Trust and Governance
- policy decision API
- replay slices and trace exports
- risk-tiered controls and approvals

## Phase WS5: Retrieval and Memory Federation
- memory indexing and ranked retrieval endpoints
- context packing with token-budget constraints
- stale/unsafe/conflict-aware filtering
- memory lifecycle retirement + GC
- retrieval feedback and evaluation metrics

## Immediate Next Tasks
1. ~~Add D1 schema migration for silver entities.~~ (done: `migrations/0001_silver_entities.sql`, D1 binding in wrangler)
2. ~~Add R2 artifact write/read path.~~ (done: PUT/GET `/v1/artifacts/:key` use R2 binding)
3. ~~Add queue consumer for enrichment jobs.~~ (done: `#[event(queue)]` handler, consumer in wrangler)
4. Implement `/mcp/task/next` with Durable Object leases.
