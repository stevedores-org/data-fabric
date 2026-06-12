# Reasoning Traces

`POST /v1/reasoning-traces` accepts structured ADK/BaseAgent step telemetry for a
single reasoning step. The endpoint is tenant-authenticated like other
`/v1/*` routes.

```json
{
  "schema_version": 1,
  "idempotency_key": "job-123:step-4",
  "agent_id": "agent-1",
  "job_id": "job-123",
  "parent_span_id": "span-parent",
  "step_number": 4,
  "step_type": "tool_call",
  "inputs": { "tool": "cargo", "args": ["test"] },
  "outputs": { "status": "ok" },
  "token_cost": {
    "input": 128,
    "output": 32,
    "cached": 64
  },
  "started_at": "2026-06-12T00:00:00Z",
  "completed_at": "2026-06-12T00:00:02Z",
  "metadata": { "source": "adk" }
}
```

The sink redacts sensitive fields in `inputs`, `outputs`, and `metadata` before
persistence. Payloads up to 1 KiB are stored inline in D1. Larger redacted
payloads are archived to the `ARTIFACTS` R2 bucket and the D1 row stores an
`r2://ARTIFACTS/...` pointer.

Clients should set `idempotency_key` to a stable per-step value. Duplicate
submissions with the same tenant and key are acknowledged without creating a
second row.

Sink failures are retried three times and logged. If persistence still fails,
the endpoint returns a non-fatal `202` response with `accepted: false` and
`Retry-After: 5`, so agent work can continue while the trace client retries.
