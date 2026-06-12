-- Reasoning traces: structured ADK/BaseAgent step telemetry.
-- Large redacted payloads are archived to the ARTIFACTS bucket and referenced here.

CREATE TABLE IF NOT EXISTS reasoning_traces (
    tenant_id TEXT NOT NULL,
    id TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    idempotency_key TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    job_id TEXT NOT NULL,
    parent_span_id TEXT,
    step_number INTEGER NOT NULL,
    step_type TEXT NOT NULL,
    inputs TEXT,
    inputs_archive_url TEXT,
    inputs_size_bytes INTEGER NOT NULL DEFAULT 0,
    outputs TEXT,
    outputs_archive_url TEXT,
    outputs_size_bytes INTEGER NOT NULL DEFAULT 0,
    token_input INTEGER NOT NULL DEFAULT 0,
    token_output INTEGER NOT NULL DEFAULT 0,
    token_cached INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    metadata TEXT,
    received_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_reasoning_traces_job_step
  ON reasoning_traces(tenant_id, job_id, step_number, started_at ASC);
CREATE INDEX IF NOT EXISTS idx_reasoning_traces_agent
  ON reasoning_traces(tenant_id, agent_id, received_at DESC);
CREATE INDEX IF NOT EXISTS idx_reasoning_traces_parent_span
  ON reasoning_traces(tenant_id, parent_span_id);
