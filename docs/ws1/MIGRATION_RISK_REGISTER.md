# WS1 Migration Risk Register

Scale:
- Severity: `HIGH | MEDIUM | LOW`
- Likelihood: `HIGH | MEDIUM | LOW`

| Risk ID | Risk | Severity | Likelihood | Trigger / detection signal | Mitigation |
|---|---|---|---|---|---|
| `R1` | Storage model drift moving from Surreal-centric patterns to Cloudflare D1/R2/KV | HIGH | HIGH | Query or migration failures during parity tests | Introduce repository abstraction with explicit D1 SQL contracts and compatibility tests per entity |
| `R2` | Access-control regression if DataMesh behavior is not reimplemented | HIGH | MEDIUM | Unauthorized read/write succeeds in integration tests | Implement policy engine before broad API rollout; enforce deny-by-default and add authz test matrix |
| `R3` | PII leakage if write path bypasses redaction checks | HIGH | MEDIUM | PII fixtures appear in Silver/Gold outputs | Add mandatory pipeline gate in write handlers; fail closed on uncertain scans; add regression fixtures |
| `R4` | Task queue lease/retry race conditions under concurrent agents | HIGH | MEDIUM | duplicate claims or stuck `running` tasks | Use atomic claim SQL, heartbeat lease extension, and dead-letter retry cutoff with chaos tests |
| `R5` | Contract mismatch between MCP clients and new worker endpoints | MEDIUM | HIGH | client parse errors or invalid method failures | Publish versioned MCP schema docs and provide compatibility shims for legacy request formats |
| `R6` | Event model divergence between orchestration runtimes and WS2 entities | MEDIUM | MEDIUM | inability to map event payloads to canonical entities | Define canonical event envelope and conversion adapters early; validate with sample traces |
| `R7` | Federated query connectors introduce latency/cost spikes | MEDIUM | MEDIUM | high tail latency, egress spikes, query timeout errors | Start with async/offline query mode + pushdown defaults + per-source timeout budgets |
| `R8` | Missing replay/audit lineage prevents debugging autonomous failures | HIGH | LOW | cannot reconstruct failed run path from stored records | Treat event + checkpoint persistence as mandatory for state transitions |
| `R9` | Multi-repo orchestrator integration breaks due to incompatible IDs/status vocabularies | MEDIUM | MEDIUM | orphaned references, failed joins across Run/Task/Plan | Adopt canonical ID and status vocabulary in adapters; add conformance checks |
| `R10` | Incremental migration creates partial feature flags with undefined behavior | MEDIUM | MEDIUM | environment-specific failures between dev/prod | Gate each capability behind explicit runtime flags and maintain rollout matrix |

## Immediate mitigation owners (proposed)

1. `R1`, `R4`: storage/task runtime maintainers.
2. `R2`, `R3`: security/policy owners.
3. `R5`, `R6`, `R9`: API/orchestration integration owners.
4. `R7`, `R10`: platform/performance owners.

## Exit criteria for WS1 risk closure

- P0 risks (`R1-R4`) have executable integration tests.
- MCP schema versioning and adapter strategy documented (`R5`).
- Canonical event envelope validated on at least one end-to-end run (`R6`, `R8`, `R9`).
