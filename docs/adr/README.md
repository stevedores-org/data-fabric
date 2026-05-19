# Architecture Decision Records

This directory stores data-fabric architecture decision records (ADRs).
ADRs are lightweight, durable records for decisions that affect runtime
architecture, data contracts, operational posture, or cross-repo integration.

## Decision Flow

1. Add a proposal from `0000-template.md` using the next sequence number.
2. Fill in context, options, scoring, decision, consequences, and rollback plan.
3. Mark the ADR `proposed` while review is active.
4. Move to `accepted`, `rejected`, or `superseded` after review.
5. Link the ADR from `BACKLOG.md` and any related issue or PR.

Low-risk decisions can be reviewed asynchronously in a PR. Workstream-blocking
or high-risk decisions should be reviewed in the weekly architecture review.

## Scoring

Score each option from 1 to 5:

| Dimension | 1 | 5 |
| --- | --- | --- |
| Impact | Small/local effect | Major velocity or platform effect |
| Risk | Low blast radius | High blast radius or hard to validate |
| Complexity | Simple implementation | Multi-system or novel implementation |
| Reversibility | Easy rollback | Hard migration or durable data impact |

Prefer options with high impact, manageable risk, low complexity, and high
reversibility. When a high-risk option is selected, the rollback plan must be
explicit enough to execute without redesign.

## Evidence Bar

Every accepted ADR should cite at least one concrete evidence source:

- benchmark or measurement
- prototype or integration test
- prior production behavior from data-fabric or an integration repo
- compatibility analysis against current D1/R2/Worker constraints
- operational rollback or migration rehearsal

## Current ADRs

| ADR | Status | Decision |
| --- | --- | --- |
| [0001](0001-event-ordering-strategy.md) | accepted | Use Hybrid Logical Clocks for event ordering |
| [0002](0002-policy-engine-placement.md) | accepted | Keep policy evaluation in-worker for the initial control plane |
| [0003](0003-memory-index-backend.md) | accepted | Start with D1 FTS plus optional external retrieval adapters |

