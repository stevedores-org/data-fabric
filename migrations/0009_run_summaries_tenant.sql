-- WS8: Add multi-tenancy to run_summaries (missed in initial migration)
-- We recreate the table to change the Primary Key to (tenant_id, run_id) for better isolation.

ALTER TABLE run_summaries RENAME TO run_summaries_old;

CREATE TABLE run_summaries (
    tenant_id TEXT NOT NULL DEFAULT 'default',
    run_id TEXT NOT NULL,
    event_count INTEGER NOT NULL DEFAULT 0,
    first_event_at TEXT,
    last_event_at TEXT,
    actors TEXT,       -- JSON array of unique actors
    event_types TEXT,  -- JSON array of unique event types
    updated_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, run_id)
);

-- Migration of existing data
INSERT INTO run_summaries (tenant_id, run_id, event_count, first_event_at, last_event_at, actors, event_types, updated_at)
SELECT 'default', run_id, event_count, first_event_at, last_event_at, actors, event_types, updated_at
FROM run_summaries_old;

DROP TABLE run_summaries_old;

CREATE INDEX IF NOT EXISTS idx_run_summaries_tenant_updated ON run_summaries(tenant_id, updated_at DESC);
