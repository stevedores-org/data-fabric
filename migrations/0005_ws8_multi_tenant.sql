-- WS8: Multi-tenant security foundation
-- Adds tenant partition keys and baseline provisioning metadata.

CREATE TABLE IF NOT EXISTS tenants (
    tenant_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    plan TEXT NOT NULL DEFAULT 'standard',
    quota_runs_per_minute INTEGER NOT NULL DEFAULT 120,
    quota_storage_bytes INTEGER NOT NULL DEFAULT 5368709120,
    federation_enabled INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

ALTER TABLE runs ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE tasks ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE plans ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE tool_calls ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE artifacts ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE policy_decisions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE releases ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE events ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE relationships ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE mcp_tasks ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE agents ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE checkpoints ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE events_bronze ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE events_silver ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE policy_rules ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';

CREATE INDEX IF NOT EXISTS idx_runs_tenant_created ON runs(tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_tasks_tenant_run ON tasks(tenant_id, run_id);
CREATE INDEX IF NOT EXISTS idx_plans_tenant_run ON plans(tenant_id, run_id);
CREATE INDEX IF NOT EXISTS idx_tool_calls_tenant_run ON tool_calls(tenant_id, run_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_tenant_run ON artifacts(tenant_id, run_id);
CREATE INDEX IF NOT EXISTS idx_policy_decisions_tenant_created ON policy_decisions(tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_releases_tenant_repo ON releases(tenant_id, repo);
CREATE INDEX IF NOT EXISTS idx_events_tenant_run ON events(tenant_id, run_id);
CREATE INDEX IF NOT EXISTS idx_mcp_tasks_tenant_status ON mcp_tasks(tenant_id, status, priority DESC, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_agents_tenant_status ON agents(tenant_id, status);
CREATE INDEX IF NOT EXISTS idx_checkpoints_tenant_thread ON checkpoints(tenant_id, thread_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_events_bronze_tenant_run ON events_bronze(tenant_id, run_id, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_events_silver_tenant_run ON events_silver(tenant_id, run_id, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_policy_rules_tenant_enabled ON policy_rules(tenant_id, enabled, priority DESC);
