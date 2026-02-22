# Data Fabric Architecture (Lean Cloudflare Build)

## Mission
Increase velocity for orchestrated autonomous AI agent builders through:
- reusable retrieval context (RAG)
- durable provenance
- low-friction task communications over data fabric

## Design Principles
- Rust-first runtime and services
- Cloudflare-native primitives for low-ops delivery
- provenance-first data model (every action traceable)
- degrade gracefully under partial outages

## Target Runtime Topology

### 1. Edge API (Cloudflare Workers, Rust)
- `/health`
- `/fabric/ingest`
- `/fabric/query`
- `/fabric/plays/{play}/launch`
- `/mcp/task/next`
- `/mcp/response`

### 2. Storage Planes
- Bronze: immutable raw events and artifacts
- Silver: normalized entities (task, run_event, memory, artifact)
- Gold: retrieval-ready context packs and summaries

Cloudflare mapping:
- D1: relational metadata and indexes
- R2: large artifacts
- KV: hot context cache
- Vectorize: semantic retrieval index

### 3. Coordination Plane
- Durable Objects: per-project/per-run cursor, idempotency, leases
- Queues: async ingest, enrichment, embedding jobs

### 4. RAG Plane
- Fast path at edge: semantic retrieval + metadata filtering
- Deep path: optional external GraphRAG adapter for complex multi-hop queries
- Model traffic routed through AI Gateway for observability and policy

## Integration Targets
- `oxidizedgraph`: workflow execution and node-level orchestration
- `aivcs`: run/spec provenance and release gating
- `oxidizedRAG`: deep retrieval and graph reasoning path

## Mission KPIs
- higher autonomous task completion
- improved first-pass CI for agent-authored changes
- reduced MTTR for failed runs
- increased reuse of prior context/artifacts
- reduced human interventions per release
