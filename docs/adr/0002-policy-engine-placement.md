# ADR-0002: Policy Engine Placement

Status: accepted
Date: 2026-05-19
Issue: #51
Workstream: WS4

## Context

WS4 policy checks sit on the request path for autonomous agents. The first
implementation needs low latency, tenant-aware decisions, and simple deployment
inside the Cloudflare Worker. The alternative is an external policy engine such
as OPA, which increases expressiveness but adds network and operational cost.

## Options

| Option | Summary | Impact | Risk | Complexity | Reversibility |
| --- | --- | ---: | ---: | ---: | ---: |
| A | In-worker Rust policy evaluator | 5 | 2 | 3 | 4 |
| B | External OPA service | 4 | 4 | 4 | 3 |
| C | Static allow/deny configuration only | 2 | 3 | 1 | 5 |

## Evidence

- Current policy code and tests already cover risk classification, wildcard
  matching, rate limits, and tenant authorization in-process.
- Worker-local policy checks avoid a network hop on every agent action.

## Decision

Keep policy evaluation in the Worker for the initial control plane. Store policy
definitions in data-fabric storage, evaluate them in Rust, and persist decisions
for audit and replay.

## Consequences

- Policy checks remain fast and deploy with the Worker.
- Policy language stays narrower than OPA/Rego until there is concrete demand.
- Future OPA integration should be adapter-based rather than a hard dependency.

## Rollback Plan

If in-worker policy becomes too limited, introduce an external evaluator behind
the same `/v1/policies/check` contract. Keep the Rust evaluator as fallback for
degraded mode and migrate policy definitions through a compatibility adapter.

