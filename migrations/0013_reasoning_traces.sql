-- Epic 3 (#110) / Task 3.2 (#111): ADK BaseAgent reasoning-trace stream sink.
--
-- One row per reasoning step. Hot path is INSERT-only; readers query by
-- (tenant_id, job_id) for replay and by (tenant_id, agent_id) for audit.
--
-- Idempotency: an agent retrying a failed POST must not double-write the same
-- step. (tenant_id, idempotency_key) is unique and the insert uses
-- ON CONFLICT DO NOTHING so the second attempt is a no-op success.
--
-- Large payloads (>1 KB) are offloaded to R2 (Cloudflare's GZRS analogue);
-- only the R2 key is stored in the row, in inputs_r2_key / outputs_r2_key.
-- Small payloads stay inline in inputs_inline / outputs_inline.

CREATE TABLE IF NOT EXISTS reasoning_traces (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL DEFAULT 'default',
    schema_version INTEGER NOT NULL DEFAULT 1,

    -- Identity
    agent_id TEXT NOT NULL,
    job_id TEXT NOT NULL,
    parent_span_id TEXT,
    step_number INTEGER NOT NULL,
    step_type TEXT NOT NULL,  -- tool_call | thought | commit | observation | error | other

    -- Payload (one of inline / r2 key, never both populated)
    inputs_inline TEXT,
    inputs_r2_key TEXT,
    outputs_inline TEXT,
    outputs_r2_key TEXT,

    -- Token cost (per-step)
    tokens_input INTEGER NOT NULL DEFAULT 0,
    tokens_output INTEGER NOT NULL DEFAULT 0,
    tokens_cached INTEGER NOT NULL DEFAULT 0,

    -- Timing
    started_at TEXT NOT NULL,
    completed_at TEXT,

    -- Idempotency: required, supplied by the BaseAgent TraceSink client
    idempotency_key TEXT NOT NULL,

    created_at TEXT NOT NULL DEFAULT (datetime('now')),

    UNIQUE (tenant_id, idempotency_key),

    -- Either inline OR archived, never both. Allowed to be neither when the
    -- step legitimately has no payload (e.g. a Thought with only token cost).
    CHECK (inputs_inline IS NULL OR inputs_r2_key IS NULL),
    CHECK (outputs_inline IS NULL OR outputs_r2_key IS NULL),

    -- Keep the step_type column honest at the storage layer so a client
    -- typo can't silently coerce to "other" on readback.
    CHECK (step_type IN ('tool_call', 'thought', 'commit', 'observation', 'error', 'other')),

    -- Non-empty idempotency_key — empty string would collide on the unique
    -- constraint after one row and turn the dedupe path into a foot-gun.
    CHECK (length(idempotency_key) > 0)
);

CREATE INDEX IF NOT EXISTS idx_reasoning_traces_job
    ON reasoning_traces (tenant_id, job_id, step_number);
CREATE INDEX IF NOT EXISTS idx_reasoning_traces_agent
    ON reasoning_traces (tenant_id, agent_id, started_at);
