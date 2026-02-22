-- M1: Foundation schema for agent orchestration
-- tasks, agents, checkpoints, events_bronze

-- ── Tasks: priority queue for agent work ─────────────────────────
CREATE TABLE mcp_tasks (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    task_type TEXT NOT NULL,
    priority INTEGER DEFAULT 0,
    status TEXT DEFAULT 'pending',
    params TEXT,
    result TEXT,
    agent_id TEXT,
    graph_ref TEXT,
    play_id TEXT,
    parent_task_id TEXT,
    retry_count INTEGER DEFAULT 0,
    max_retries INTEGER DEFAULT 3,
    lease_expires_at TEXT,
    created_at TEXT NOT NULL,
    completed_at TEXT
);
CREATE INDEX idx_mcp_tasks_claimable ON mcp_tasks(status, priority DESC, created_at ASC);
CREATE INDEX idx_mcp_tasks_job ON mcp_tasks(job_id);
CREATE INDEX idx_mcp_tasks_agent ON mcp_tasks(agent_id);

-- ── Agents: registration ─────────────────────────────────────────
CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    capabilities TEXT NOT NULL,
    endpoint TEXT,
    last_heartbeat TEXT,
    status TEXT DEFAULT 'active',
    metadata TEXT
);

-- ── Checkpoints: oxidizedgraph state snapshots ───────────────────
CREATE TABLE checkpoints (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    parent_id TEXT,
    state_r2_key TEXT NOT NULL,
    state_size_bytes INTEGER,
    metadata TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_cp_thread ON checkpoints(thread_id, created_at DESC);

-- ── Events Bronze: raw immutable event log ───────────────────────
CREATE TABLE events_bronze (
    id TEXT PRIMARY KEY,
    run_id TEXT,
    thread_id TEXT,
    event_type TEXT NOT NULL,
    node_id TEXT,
    actor TEXT,
    payload TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_events_run ON events_bronze(run_id, created_at ASC);
CREATE INDEX idx_events_thread ON events_bronze(thread_id, created_at ASC);
CREATE INDEX idx_events_type ON events_bronze(event_type);
