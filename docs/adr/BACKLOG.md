# ADR Backlog

This backlog tracks workstream-blocking architecture decisions from #51.

| Priority | Decision | Workstream | Owner | Target ADR | Status |
| --- | --- | --- | --- | --- | --- |
| High | Event ordering strategy (HLC vs vector clocks) | WS3 | architecture review | [0001](0001-event-ordering-strategy.md) | accepted |
| High | Policy engine placement (in-worker vs external OPA) | WS4 | architecture review | [0002](0002-policy-engine-placement.md) | accepted |
| Medium | Memory index backend (D1 FTS vs external) | WS5 | architecture review | [0003](0003-memory-index-backend.md) | accepted |
| Medium | Tenant isolation model (row-level vs schema-level) | WS8 | unassigned | TBD | proposed |
| Low | SDK language priorities (Rust-first vs polyglot) | WS9 | unassigned | TBD | proposed |

## Review Cadence

- Weekly architecture review handles high-risk or workstream-blocking ADRs.
- Low-risk ADRs can be accepted asynchronously through PR review.
- Any accepted ADR can be superseded by a later ADR when measurements change.

