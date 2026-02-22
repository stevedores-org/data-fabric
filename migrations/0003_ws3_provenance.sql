-- WS3: Provenance — silver layer and trace support (issue #43)
-- Silver: own id (PK), bronze_id (FK to events_bronze.id), normalized_at, entity_refs.

-- ── Events Silver: normalized event layer ───────────────────────
CREATE TABLE IF NOT EXISTS events_silver (
    id TEXT PRIMARY KEY,
    bronze_id TEXT NOT NULL,
    run_id TEXT,
    thread_id TEXT,
    event_type TEXT NOT NULL,
    node_id TEXT,
    actor TEXT,
    payload TEXT,
    created_at TEXT NOT NULL,
    normalized_at TEXT NOT NULL,
    entity_refs TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_silver_bronze ON events_silver(bronze_id);
CREATE INDEX IF NOT EXISTS idx_events_silver_run ON events_silver(run_id, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_events_silver_thread ON events_silver(thread_id, created_at ASC);
