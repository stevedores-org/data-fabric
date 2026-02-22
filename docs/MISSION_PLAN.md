# Mission Plan

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

## Immediate Next Tasks
1. Add D1 schema migration for silver entities.
2. Add R2 artifact write/read path.
3. Add queue consumer for enrichment jobs.
4. Implement `/mcp/task/next` with Durable Object leases.
