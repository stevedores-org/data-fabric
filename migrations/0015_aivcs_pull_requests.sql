-- AIVCS Gold projection for review queue / pull request workspace reads.
-- Source of truth starts as runs.metadata.aivcs.change_set.

CREATE TABLE IF NOT EXISTS gold_aivcs_review_queue (
    tenant_id TEXT NOT NULL,
    id TEXT NOT NULL,
    repo TEXT NOT NULL,
    run_id TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    source_branch TEXT,
    target_branch TEXT,
    author TEXT,
    summary TEXT,
    change_set TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, id),
    FOREIGN KEY (run_id) REFERENCES runs(id)
);

CREATE INDEX IF NOT EXISTS idx_gold_aivcs_review_queue_tenant_updated
    ON gold_aivcs_review_queue(tenant_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_gold_aivcs_review_queue_tenant_repo_status
    ON gold_aivcs_review_queue(tenant_id, repo, status);
