-- Telemetry Snapshots: Structured performance logs for AI agents
CREATE TABLE IF NOT EXISTS telemetry_snapshots (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL DEFAULT 'default',
    agent_name TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    status TEXT NOT NULL,
    duration_seconds INTEGER NOT NULL,
    total_attempts INTEGER NOT NULL,
    success_rate REAL NOT NULL,
    namespace TEXT NOT NULL,
    payload TEXT, -- Raw JSON payload for future extensibility
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_telemetry_tenant_type ON telemetry_snapshots(tenant_id, agent_type);
CREATE INDEX IF NOT EXISTS idx_telemetry_agent_name ON telemetry_snapshots(agent_name);
