# WS5: Retrieval and Memory Federation

Implemented endpoints:

- `POST /v1/memory/index`  
  Index a reusable memory item (checkpoint/artifact/decision/context/run summary).
- `POST /v1/memory/retrieve`  
  Ranked retrieval with freshness, unsafe, and conflict filtering.
- `POST /v1/memory/context-pack`  
  Token-budgeted packing of top-ranked memories.
- `POST /v1/memory/:id/retire`  
  Explicit lifecycle transition to retired.
- `POST /v1/memory/gc`  
  Garbage-collect retired/expired memory records.
- `POST /v1/memory/retrieval-feedback`  
  Log outcome signals for evaluation.
- `GET /v1/memory/eval/summary`  
  Retrieve aggregate quality metrics (cache hit/success/first-pass + p50/p95 latency).

Schema:

- `memory_index`
- `memory_retrieval_queries`
- `memory_retrieval_feedback`

Design choices:

- Default-safe retrieval:
  - stale items filtered unless `include_stale=true`
  - unsafe items filtered unless `include_unsafe=true`
  - conflicted versions filtered unless `include_conflicted=true`
- Ranking score combines:
  - lexical similarity
  - freshness decay
  - prior success rate
  - access popularity
- Cross-repo federation:
  - retrieve from `repo` + `related_repos`
