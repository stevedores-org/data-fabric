-- AIVCS branches projection (issue #148).
--
-- Tracks branches for a repository.

CREATE TABLE IF NOT EXISTS branch (
    tenant_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    repo TEXT NOT NULL,
    name TEXT NOT NULL,
    head_sha TEXT NOT NULL,
    agent_owner TEXT,
    status TEXT NOT NULL, -- active | merged | abandoned
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, id)
);
CREATE INDEX IF NOT EXISTS idx_branch_tenant_repo
    ON branch(tenant_id, repo);
