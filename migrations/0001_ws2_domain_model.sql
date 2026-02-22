-- WS2: Domain Model and Ontology — D1 schema v1
-- Canonical entities for autonomous agent-builder data fabric.

-- ── Runs ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'created',
    trigger TEXT,
    actor TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT  -- JSON
);

-- ── Tasks ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id),
    plan_id TEXT,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'created',
    actor TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT  -- JSON
);
CREATE INDEX IF NOT EXISTS idx_tasks_run_id ON tasks(run_id);

-- ── Plans ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS plans (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id),
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'created',
    task_ids TEXT NOT NULL DEFAULT '[]',  -- JSON array
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT  -- JSON
);
CREATE INDEX IF NOT EXISTS idx_plans_run_id ON plans(run_id);

-- ── Tool Calls ──────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id),
    task_id TEXT REFERENCES tasks(id),
    tool_name TEXT NOT NULL,
    input TEXT NOT NULL,      -- JSON
    output TEXT,              -- JSON
    status TEXT NOT NULL DEFAULT 'created',
    duration_ms INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_tool_calls_run_id ON tool_calls(run_id);
CREATE INDEX IF NOT EXISTS idx_tool_calls_task_id ON tool_calls(task_id);

-- ── Artifacts ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id),
    key TEXT NOT NULL,
    content_type TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    checksum TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT  -- JSON
);
CREATE INDEX IF NOT EXISTS idx_artifacts_run_id ON artifacts(run_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_artifacts_key ON artifacts(key);

-- ── Policy Decisions ────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS policy_decisions (
    id TEXT PRIMARY KEY,
    run_id TEXT REFERENCES runs(id),
    action TEXT NOT NULL,
    actor TEXT NOT NULL,
    resource TEXT,
    decision TEXT NOT NULL,  -- 'allow', 'deny', 'escalate'
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    context TEXT  -- JSON
);
CREATE INDEX IF NOT EXISTS idx_policy_decisions_run_id ON policy_decisions(run_id);

-- ── Releases ────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS releases (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    version TEXT NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(id),
    artifact_ids TEXT NOT NULL DEFAULT '[]',  -- JSON array
    status TEXT NOT NULL DEFAULT 'created',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT  -- JSON
);
CREATE INDEX IF NOT EXISTS idx_releases_repo ON releases(repo);

-- ── Events (append-only provenance) ─────────────────────────────
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id),
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    payload TEXT  -- JSON
);
CREATE INDEX IF NOT EXISTS idx_events_run_id ON events(run_id);
CREATE INDEX IF NOT EXISTS idx_events_entity ON events(entity_kind, entity_id);

-- ── Relationships ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rel_type TEXT NOT NULL,  -- 'causality', 'dependency', 'ownership', 'lineage'
    from_kind TEXT NOT NULL,
    from_id TEXT NOT NULL,
    to_kind TEXT NOT NULL,
    to_id TEXT NOT NULL,
    relation TEXT,  -- extra qualifier (e.g., 'spawned', 'derived_from')
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_rel_from ON relationships(from_kind, from_id);
CREATE INDEX IF NOT EXISTS idx_rel_to ON relationships(to_kind, to_id);
CREATE INDEX IF NOT EXISTS idx_rel_type ON relationships(rel_type);
