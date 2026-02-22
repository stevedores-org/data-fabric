-- WS5: Retrieval & Memory — cross-run context reuse (#45)
-- Searchable memory index: scope/key + refs to runs, artifacts, checkpoints.

CREATE TABLE IF NOT EXISTS memory (
    id TEXT PRIMARY KEY,
    run_id TEXT,
    thread_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    key TEXT NOT NULL,
    ref_type TEXT NOT NULL,
    ref_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_memory_thread ON memory(thread_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_memory_run ON memory(run_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_memory_scope_key ON memory(scope, key);
