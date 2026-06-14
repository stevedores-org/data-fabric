-- AIVCS CI check run projection (issue #148).
--
-- Tracks CI checks for a change_set.

CREATE TABLE IF NOT EXISTS ci_check_run (
    tenant_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    change_set_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL, -- queued | in_progress | completed
    conclusion TEXT,      -- success | failure | neutral | cancelled | timed_out | action_required | skipped
    url TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, id)
);
CREATE INDEX IF NOT EXISTS idx_ci_check_run_tenant_change_set
    ON ci_check_run(tenant_id, change_set_id);
