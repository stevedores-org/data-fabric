# WS10 Pilot Metrics Endpoint

Issue: [#105](https://github.com/stevedores-org/data-fabric/issues/105)
Parent: [#50](https://github.com/stevedores-org/data-fabric/issues/50)
Status: implemented (draft PR)

This note describes the WS10 baseline-metrics surface used during the Phase 1
oxidizedgraph pilot. The goal is to give the pilot runbook a single
HTTP call that returns every KPI defined in the workload spec
(`docs/ws10/PHASE1_WORKLOAD.md`) plus an out-of-band shell script that
pulls Worker-platform p50/p95/p99 latency over the same window.

## Endpoint

```
GET /v1/metrics/pilot?window=1d&task_type=oxidizedgraph.test
Authorization: <tenant token>
```

The endpoint is tenant-scoped via the existing `tenant::tenant_from_request`
extractor — the same machinery used by every `/v1/...` route — so the
returned KPIs only ever cover rows visible to the calling tenant.

### Query parameters

| param       | required | default | shape          | notes                                                              |
|-------------|----------|---------|----------------|--------------------------------------------------------------------|
| `window`    | no       | `1d`    | `N(s|m|h|d)`   | Bounded to `[60s, 30d]`. Anything outside the bounds returns 400.  |
| `task_type` | no       | none    | string         | When set, restricts the task-completion-rate denominator to this task type. The other KPIs intentionally stay tenant-wide so the throughput / MTTR / intervention numbers reflect the full agent surface during the pilot. |

`parse_window` is implemented in `src/metrics.rs` and unit-tested.

### Response

```json
{
  "window": "1d",
  "window_seconds": 86400,
  "sample_counts": { "tasks": 142, "events": 8910, "decisions": 37 },
  "kpis": {
    "task_completion_rate": 0.82,
    "mttr_p50_seconds": 187.0,
    "mttr_p95_seconds": 412.0,
    "context_reuse_rate": null,
    "human_intervention_rate": 0.18,
    "event_throughput_per_sec": 0.1031
  },
  "reasons": {
    "context_reuse_rate": "no cache-hit signal in run_summaries v1"
  },
  "meta": {
    "generated_at": "2026-06-10T18:45:12.014Z",
    "tenant_id": "stevedores"
  }
}
```

The contract that pilot tooling can rely on:

- Every KPI key is always present in `kpis`. The value is either a number,
  or `null`.
- `null` is **never** a sentinel for "compute returned zero". It always means
  "the KPI could not be computed for this window/tenant". The reason is
  always populated in `reasons` under the same key.
- Zero is a legitimate KPI value (e.g. zero escalations is a real number,
  not a missing signal).
- `Server-Timing: total;dur=<ms>` is set on the response, matching the rest
  of the gold-layer endpoints.

## KPI sources

| KPI                          | Source table       | Aggregation                                                                          | Null reason if missing                                          |
|------------------------------|--------------------|--------------------------------------------------------------------------------------|-----------------------------------------------------------------|
| `task_completion_rate`       | `mcp_tasks`        | `COUNT(status='completed') / COUNT(*)` over window, optionally filtered by task_type | `no mcp_tasks rows in window`                                   |
| `mttr_p50_seconds`           | `events_bronze`    | `LEAD()` over `(run_id ORDER BY created_at)`; pair `event_type LIKE '%fail%'` with the next `event_type LIKE '%complet%'`; delta in seconds; p50 by `ORDER BY ... LIMIT 1 OFFSET floor((n-1)*0.5)` | `no failed->completed event pairs in window`                    |
| `mttr_p95_seconds`           | `events_bronze`    | same as p50 with `OFFSET floor((n-1)*0.95)`                                          | `no failed->completed event pairs in window`                    |
| `context_reuse_rate`         | `run_summaries`    | The v1 schema does not record a checkpoint hit/miss column. Hard-coded `null`.       | `no cache-hit signal in run_summaries v1`                       |
| `human_intervention_rate`    | `policy_decisions` | `COUNT(decision='escalate') / COUNT(*)` over window                                  | `no policy_decisions rows in window`                            |
| `event_throughput_per_sec`   | `events_bronze`    | `COUNT(*) / window_seconds`                                                          | `no events_bronze rows in window`                               |

### `context_reuse_rate` — known limitation

The issue spec calls this KPI out as deliberately fuzzy. The pragmatic
definition we agreed on is:

> `checkpoint reads with a prior checkpoint for the same run/node /
> total checkpoint reads`

`run_summaries` v1 does not record a hit/miss signal, and we cannot
introduce one in this PR without touching the schema (out of scope per
the issue). The endpoint therefore returns `null` with the explicit
reason `"no cache-hit signal in run_summaries v1"`. A follow-up issue
should add a column or a separate `checkpoint_reads` table.

## Platform latency: `scripts/pilot-latency.sh`

The metrics endpoint reports **fabric-side business KPIs** read from D1.
It deliberately does not report HTTP request latency, because that is a
Worker-platform metric the fabric itself does not measure on the hot
path.

For the pilot baseline, `scripts/pilot-latency.sh` calls
`bunx wrangler@3 analytics` over the same window and prints p50/p95/p99
of API request latency as a single JSON envelope:

```
scripts/pilot-latency.sh 1d
{
  "worker": "data-fabric-worker",
  "window": "1d",
  "window_seconds": 86400,
  "since": "2026-06-09T18:45:00Z",
  "until": "2026-06-10T18:45:00Z",
  "api_latency_p50_ms": 18,
  "api_latency_p95_ms": 74,
  "api_latency_p99_ms": 192,
  "raw": { ... }
}
```

The script mirrors `parse_window` in pure bash so the windows always
agree with the endpoint. The wrangler CLI surface is volatile across
versions; the script handles two known invocation shapes and falls back
with a clear error message if Cloudflare changes it again.

## Operational notes

- All five D1 queries are tenant-scoped via `WHERE tenant_id = ?1` and
  use the existing indexes (`idx_mcp_tasks_tenant_status`,
  `idx_events_bronze_tenant_run`, `idx_policy_decisions_tenant_created`).
- D1 supports SQLite window functions, which is what the MTTR query
  relies on (`LEAD() OVER (PARTITION BY ... ORDER BY ...)`).
- The `since` predicate uses `strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-N seconds')`
  so the comparison is lexicographic-safe against the ISO-8601 strings
  the fabric writes via `db::now_iso()`.
- The endpoint runs three round-trips to D1 in the worst case: one query
  to count failure->completion pairs, then two ordered LIMIT/OFFSET reads
  to pick p50 and p95. This keeps each statement under the per-statement
  row budget, at the cost of a small amount of extra work.

## Test surface

- `parse_window` is exercised by four unit tests:
  - canonical units (`Nd`, `Nh`, `Nm`, `Ns`)
  - whitespace trimming
  - invalid input rejection
  - bounds rejection (below 60s and above 30d)
- `aggregate_kpis` is exercised by three unit tests over in-memory
  `RawAggregates` fixtures:
  - happy path with all denominators present
  - empty window (every KPI `null` with a reason)
  - partial window (only `task_completion_rate` real, rest `null` with reasons)

These are pure-Rust tests and run under the standard `cargo test --lib`
gate.
