# ADR-0003: Memory Index Backend

Status: accepted
Date: 2026-05-19
Issue: #51
Workstream: WS5

## Context

WS5 retrieval and memory federation need a default index that works in the
Cloudflare-native deployment while still allowing deeper retrieval integrations
for graph or vector-heavy workloads.

## Options

| Option | Summary | Impact | Risk | Complexity | Reversibility |
| --- | --- | ---: | ---: | ---: | ---: |
| A | D1 FTS as the default index | 4 | 2 | 2 | 4 |
| B | External vector database as the default | 5 | 4 | 4 | 3 |
| C | Adapter-only, no built-in index | 3 | 3 | 2 | 5 |

## Evidence

- The architecture already maps metadata and indexes to D1, with optional deep
  retrieval adapters for more complex graph reasoning.
- A built-in D1 path keeps local development and tenant bootstrap simple.

## Decision

Use D1 FTS as the default memory index for first-party retrieval. Keep the
adapter boundary open for external vector or graph backends when a workload
needs deeper semantic retrieval than D1 FTS can provide.

## Consequences

- The default deployment remains Cloudflare-native and low-ops.
- Retrieval quality is bounded by D1 FTS until adapter-backed semantic retrieval
  is configured.
- WS5 should document when to route a query to an external retrieval adapter.

## Rollback Plan

If D1 FTS is not sufficient for pilot workloads, promote the external retrieval
adapter to the default path while retaining D1 FTS as a metadata and fallback
index. Existing memory rows remain valid because the index backend is an
implementation detail behind the retrieval API.

