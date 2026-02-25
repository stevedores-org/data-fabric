-- WS3 Gold Layer: Task Dependencies (issue #58)
CREATE TABLE IF NOT EXISTS task_dependencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tenant_id TEXT NOT NULL DEFAULT 'default',
    run_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    depends_on_task_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_taskdep_unique
  ON task_dependencies(run_id, task_id, depends_on_task_id);
CREATE INDEX IF NOT EXISTS idx_taskdep_tenant_run
  ON task_dependencies(tenant_id, run_id);
