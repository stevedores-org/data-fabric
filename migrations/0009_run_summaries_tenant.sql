-- WS3 Gold Layer: Tenant-aware Run Summaries
DROP TABLE IF EXISTS run_summaries;
CREATE TABLE run_summaries (
    tenant_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    event_count INTEGER NOT NULL DEFAULT 0,
    first_event_at TEXT,
    last_event_at TEXT,
    actors TEXT,       -- JSON array of unique actors
    event_types TEXT,  -- JSON array of unique event types
    updated_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, run_id)
);
CREATE INDEX IF NOT EXISTS idx_run_summaries_tenant_updated
  ON run_summaries(tenant_id, updated_at DESC);
