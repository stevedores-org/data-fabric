-- WS5: Retrieval and Memory Federation
-- Searchable memory index, retrieval telemetry, and evaluation feedback.

CREATE TABLE IF NOT EXISTS memory_index (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    kind TEXT NOT NULL,
    run_id TEXT,
    task_id TEXT,
    thread_id TEXT,
    checkpoint_id TEXT,
    artifact_key TEXT,
    title TEXT,
    summary TEXT NOT NULL,
    tags TEXT,                 -- JSON array
    content_ref TEXT,          -- R2 key or external reference
    metadata TEXT,             -- JSON object
    success_rate REAL,         -- 0.0..1.0
    source_created_at TEXT,
    indexed_at TEXT NOT NULL,
    last_accessed_at TEXT,
    access_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'active', -- active|retired
    unsafe_reason TEXT,
    expires_at TEXT,
    conflict_key TEXT,
    conflict_version INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_memory_repo_status ON memory_index(repo, status);
CREATE INDEX IF NOT EXISTS idx_memory_thread ON memory_index(thread_id, indexed_at DESC);
CREATE INDEX IF NOT EXISTS idx_memory_run ON memory_index(run_id, indexed_at DESC);
CREATE INDEX IF NOT EXISTS idx_memory_expires ON memory_index(expires_at);
CREATE INDEX IF NOT EXISTS idx_memory_conflict ON memory_index(conflict_key, conflict_version DESC);

CREATE TABLE IF NOT EXISTS memory_retrieval_queries (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    query_text TEXT NOT NULL,
    run_id TEXT,
    task_id TEXT,
    thread_id TEXT,
    top_k INTEGER NOT NULL,
    related_repos TEXT,        -- JSON array
    returned_count INTEGER NOT NULL,
    latency_ms INTEGER NOT NULL,
    stale_filtered INTEGER NOT NULL DEFAULT 0,
    unsafe_filtered INTEGER NOT NULL DEFAULT 0,
    conflict_filtered INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_retrieval_queries_repo ON memory_retrieval_queries(repo, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_retrieval_queries_latency ON memory_retrieval_queries(latency_ms);

CREATE TABLE IF NOT EXISTS memory_retrieval_feedback (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    query_id TEXT NOT NULL,
    run_id TEXT,
    task_id TEXT,
    success INTEGER NOT NULL,            -- 0 or 1
    first_pass_success INTEGER NOT NULL, -- 0 or 1
    cache_hit INTEGER NOT NULL,          -- 0 or 1
    latency_ms INTEGER,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_feedback_query ON memory_retrieval_feedback(query_id);
CREATE INDEX IF NOT EXISTS idx_feedback_created ON memory_retrieval_feedback(created_at DESC);
