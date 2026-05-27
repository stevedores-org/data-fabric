# ADR-0001: Event Ordering Strategy

Status: accepted
Date: 2026-05-19
Issue: #51
Workstream: WS3

## Context

WS3 provenance needs stable ordering across Worker requests, D1 writes, queue
consumers, and integration adapters. The ordering strategy must preserve enough
causality for trace reconstruction without forcing every producer to coordinate
through a single clock authority.

## Options

| Option | Summary | Impact | Risk | Complexity | Reversibility |
| --- | --- | ---: | ---: | ---: | ---: |
| A | Wall-clock timestamps only | 2 | 2 | 1 | 5 |
| B | Hybrid Logical Clocks (HLC) | 5 | 3 | 3 | 4 |
| C | Vector clocks per actor | 5 | 4 | 5 | 2 |

## Evidence

- Current event and provenance models already carry timestamps and actor/run
  identity, so HLC can be added without replacing the whole event schema.
- Vector clocks give richer causality but add payload size and merge complexity
  for every integration producer.

## Decision

Use Hybrid Logical Clocks for canonical event ordering. Persist HLC fields
alongside existing event timestamps where strict replay ordering is required.
Use wall-clock timestamps for display and analytics only.

## Consequences

- Trace reconstruction can order concurrent events deterministically.
- Producers need a small HLC helper but do not need full vector-clock state.
- Cross-system adapters can degrade to wall-clock plus sequence number until
  they support HLC metadata.

## Rollback Plan

If HLC proves insufficient, add vector-clock metadata as an optional extension
without removing existing HLC columns. Consumers should treat unknown causality
fields as advisory until a superseding ADR changes the canonical contract.

