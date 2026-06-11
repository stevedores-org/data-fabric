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
| `mttr_p50_seconds` / `mttr_p95_seconds` | `events_bronze` | For each failure event (`run_failed` / `task_failed` / `error`), delta in seconds to the next recovery event (`run_completed` / `task_completed` / `recovered`) for the same `run_id`. p50/p95 are linear-interpolation percentiles of the resulting deltas. Up to 50,000 most-recent rows are sampled. | Null when no failure→recovery transitions in window. Cross-run boundaries are not crossed. Above 50,000 events, p50/p95 are approximate. |
| `context_reuse_rate` | `events_bronze` (stopgap) | `COUNT(payload.hit ∈ {true, "true"}) / COUNT(*)` over rows where `event_type = 'checkpoint_read'`. Predicate is `json_extract(payload, '$.hit') IN (1, 'true', 'True', 'TRUE')`. | **Stopgap definition** — see "Stopgap notes" below. Null when no checkpoint_read events in window. |
| `human_intervention_rate` | `policy_decisions` | `COUNT(decision='escalate') / COUNT(*)` over window. | Null when no decisions in window. |
| `event_throughput_per_sec` | `events_bronze` | `COUNT(*) / window_seconds`. | Null when no events in window. **Caveat:** averaged over the *requested* window, not the active-data window — if the tenant's first ingest was N seconds ago and `window_seconds > N`, throughput under-reports by `~N / window_seconds`. |

### Stopgap notes — `context_reuse_rate`

The #105 risk section flagged this KPI as the fuzziest of the six: no single table directly counts cache hits vs misses. This PR defines it deliberately so the KPI returns a real number now and pilot-week reviewers can sanity-check it:

- **Numerator:** rows in `events_bronze` with `event_type = 'checkpoint_read'` and a payload where `payload.hit` evaluates to JSON `true`.
- **Denominator:** rows in `events_bronze` with `event_type = 'checkpoint_read'`.
- **Producer contract:** code that reads a checkpoint must emit a `checkpoint_read` event with `{"hit": true|false, ...}` in the payload. (No producer guarantees this yet — payloads written without a `hit` field count as misses, which is the conservative default.)

If reviewers prefer a different definition (e.g., reads-against-writes from `run_summaries`), point me at it and I'll swap the query. Either way, this is a Phase-1 KPI that we expect to refine before Phase 2 gates.

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

## Smoke test

`scripts/pilot-smoke.sh` calls the endpoint with `curl`, asserts HTTP 200, and verifies the response has the five required top-level keys (`window`, `window_seconds`, `sample_counts`, `kpis`, `meta`) plus all six KPIs in `kpis`. Use it as a reviewer / post-deploy sanity check.

```bash
# Local dev
scripts/pilot-smoke.sh --tenant-id acme

# Staging
scripts/pilot-smoke.sh --base-url https://data-fabric.stevedores.org --tenant-id acme --window 7d

# With task_type filter
scripts/pilot-smoke.sh --tenant-id acme --task-type oxidizedgraph.test
```

Exit codes: `0` shape valid, `2` missing prereqs, `3` request failed / non-200, `4` 200 but shape invalid.

## API latency: how it gets into Analytics Engine

The Worker emits one data point per non-public request **that passes tenant authz** to the `PILOT_LATENCY` Analytics Engine binding (see `emit_pilot_latency` in `src/lib.rs`). Pre-router 401 / 403 responses are not sampled — they short-circuit before the emit path. Schema:

| Field | Value |
|---|---|
| `index1` | `tenant_id` (sampling key) |
| `blob1` | request path (raw — high-cardinality routes like `/v1/runs/:id` inflate the dataset; templating is a follow-up) |
| `blob2` | HTTP method |
| `blob3` | `APP_ENV` (`dev` / `staging` / `production` — for cross-env filtering in shared dashboards) |
| `double1` | elapsed milliseconds |
| `double2` | response status code |

Emission is best-effort: a missing binding or transient sink failure must not turn a successful request into a 500.

`scripts/pilot-latency.sh` reads `double1` from this dataset to compute p50/p95/p99.

## Implementation map

| File | Role |
|---|---|
| `src/metrics.rs` | KPI computation. Split into `query_inputs` (D1 IO → `PilotInputs`) + `assemble` (pure → `PilotMetrics`). Pure helpers (`parse_window`, `percentile_sorted`, `extract_mttr_deltas_seconds`) plus the assembly branches are unit-tested in-source. |
| `src/lib.rs` (route `/v1/metrics/pilot`) | Parses query params, resolves tenant context, delegates to `metrics::pilot`. |
| `src/lib.rs` (`emit_pilot_latency`) | Per-request `writeDataPoint` to the `PILOT_LATENCY` binding. Best-effort. |
| `scripts/pilot-smoke.sh` | Reviewer / deploy sanity check against the live endpoint. |
| `scripts/pilot-latency.sh` | Workers Analytics Engine query for p50/p95/p99 latency. |
| `wrangler.toml` (`[[analytics_engine_datasets]]`) | `PILOT_LATENCY` binding per env. |
