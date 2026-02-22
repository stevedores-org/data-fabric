-- WS3: Provenance Complete — gold layer run summaries (issue #43)
-- Materialized run summaries for fast overview queries.

-- ── Run Summaries (gold layer) ──────────────────────────────────
CREATE TABLE IF NOT EXISTS run_summaries (
    run_id TEXT PRIMARY KEY,
    event_count INTEGER NOT NULL DEFAULT 0,
    first_event_at TEXT,
    last_event_at TEXT,
    actors TEXT,       -- JSON array of unique actors
    event_types TEXT,  -- JSON array of unique event types
    updated_at TEXT NOT NULL
);
