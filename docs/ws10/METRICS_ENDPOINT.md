# WS10 baseline metrics — `/v1/metrics/pilot`

Sub-task of [#50](https://github.com/stevedores-org/data-fabric/issues/50); implements [#105](https://github.com/stevedores-org/data-fabric/issues/105).

This is the data plane every Phase 1 go/no-go gate reads. Five KPIs come from D1; the sixth (API latency) is queried separately via Workers Analytics Engine.

## Endpoint

```
GET /v1/metrics/pilot?window=1d&task_type=oxidizedgraph.test
x-tenant-id: <required>
x-tenant-role: viewer | builder | admin   (any is fine; GET is allowed for all)
```

Query parameters:

| Param       | Required | Default | Notes                                                 |
|-------------|----------|---------|-------------------------------------------------------|
| `window`    | no       | `1d`    | `Nh` / `Nd` / `Nw`. `24h` and `1d` are equivalent.    |
| `task_type` | no       | —       | Filters `task_completion_rate` only (see below).      |

400 with `INVALID_WINDOW` if `window` doesn't parse. 401 if `x-tenant-id` is missing.

## Response

```json
{
  "window": "1d",
  "window_seconds": 86400,
  "sample_counts": { "tasks": 142, "events": 8910, "decisions": 37 },
  "kpis": {
    "task_completion_rate": 0.82,
    "mttr_p50_seconds": 187,
    "mttr_p95_seconds": 412,
    "context_reuse_rate": null,
    "human_intervention_rate": 0.18,
    "event_throughput_per_sec": 0.103
  },
  "null_reasons": {
    "context_reuse_rate": "definition deferred — see issue #105 risk section"
  },
  "meta": { "generated_at": "2026-05-25T...", "tenant_id": "..." }
}
```

`null_reasons` is omitted entirely when no KPI is null. `sample_counts` is always present — small denominators flip go/no-go decisions silently otherwise.

## KPI definitions

| KPI | Source | Computation | Notes |
|---|---|---|---|
| `task_completion_rate` | `mcp_tasks` | `COUNT(status='completed') / COUNT(*)` over window. Honors `task_type`. | Null when no tasks in window. |
| `mttr_p50_seconds` / `mttr_p95_seconds` | `events_bronze` | For each failure event (`run_failed` / `task_failed` / `error`), delta in seconds to the next recovery event (`run_completed` / `task_completed` / `recovered`) for the same `run_id`. p50/p95 are linear-interpolation percentiles of the resulting deltas. | Null when no failure→recovery transitions in window. Cross-run boundaries are not crossed. |
| `context_reuse_rate` | (deferred) | — | **Always null.** See [#105 risk section](https://github.com/stevedores-org/data-fabric/issues/105) — needs a concrete hit/miss definition first. |
| `human_intervention_rate` | `policy_decisions` | `COUNT(decision='escalate') / COUNT(*)` over window. | Null when no decisions in window. |
| `event_throughput_per_sec` | `events_bronze` | `COUNT(*) / window_seconds`. | Null when no events in window. |

### Null semantics

A KPI is **null with a reason** when its denominator is zero (no data). Zero is reserved for "denominator non-zero, numerator was actually zero." A pilot-week gate that reads `0` would silently pass; one that reads `null + reason` knows to wait for data.

### `task_type` scope

`task_type` filters `task_completion_rate` only. The other four D1 KPIs are scoped by `tenant_id` and `window` but not by task type — `events_bronze` and `policy_decisions` don't carry `task_type` directly, and joining through `mcp_tasks → run_id` would change the semantics in subtle ways. If callers need a fully task-type-scoped slice, file a follow-up issue.

## KPI #6: API latency (separate path)

The latency KPI lives in Workers Analytics Engine, not D1, so it's read by a script that calls the Analytics Engine SQL API:

```bash
export CLOUDFLARE_API_TOKEN=...   # scope: Account Analytics: Read
scripts/pilot-latency.sh --window 1d
```

Output (on success):

```json
{
  "window": "1d",
  "dataset": "data_fabric_pilot_latency",
  "sample_count": 12345,
  "p50_ms": 47.2,
  "p95_ms": 184.0,
  "p99_ms": 612.0
}
```

Exit codes: `0` success, `2` missing prereqs, `3` API error, `4` dataset empty.

### Wrangler binding

`wrangler.toml` declares an Analytics Engine dataset binding (`PILOT_LATENCY` → `data_fabric_pilot_latency` in prod, `_staging`/`_dev` variants per env). The dataset is **created on first `writeDataPoint` call**, so no pre-provisioning is needed.

> **Caveat:** the Worker is not yet emitting to this dataset. Until it does, `scripts/pilot-latency.sh` exits with code 4 ("no rows in dataset"). Emitting `latency_ms` per request is the next WS10 follow-up.

## Implementation map

| File | Role |
|---|---|
| `src/metrics.rs` | KPI computation; pure-Rust helpers (`parse_window`, `percentile_sorted`, `extract_mttr_deltas_seconds`) are unit-tested in-source. |
| `src/lib.rs` (route `/v1/metrics/pilot`) | Parses query params, resolves tenant context, delegates to `metrics::pilot`. |
| `scripts/pilot-latency.sh` | Workers Analytics Engine query for p50/p95/p99 latency. |
| `wrangler.toml` (`[[analytics_engine_datasets]]`) | `PILOT_LATENCY` binding per env. |
