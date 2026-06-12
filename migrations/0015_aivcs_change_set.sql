-- AIVCS slice 1: native `change_set` entity (the projection above the diff
-- artifact). Per issue #148 ("AIVCS UI as a human-facing control plane on
-- top of data-fabric"), AIVCS needs a first-class entity that represents a
-- proposed change — an agent's PR-equivalent — with status, risk, confidence,
-- and pointers to the diff/summary artifacts in R2 (`*_artifact_key`) and the
-- originating run (`run_id`).
--
-- The issue notes this concept "can begin as Run.metadata.aivcs.change_set,
-- then graduate into a table". This migration is that graduation: a stand-
-- alone, multi-tenant-safe projection table with tenant_id as the leading
-- column of the composite PRIMARY KEY (consistent with the WS8 isolation
-- model — see 0005_ws8_multi_tenant.sql, 0014_ws8_tenant_id_addendum.sql).
--
-- Scope of this slice: projection + types + DB layer only. HTTP routes
-- (POST/GET /v1/aivcs/change-sets), gold read-models, and the BFF surface
-- are explicitly out of scope and land in subsequent slices.

CREATE TABLE IF NOT EXISTS change_set (
  tenant_id TEXT NOT NULL DEFAULT '',
  id TEXT NOT NULL,
  repo TEXT NOT NULL,
  base_ref TEXT NOT NULL,
  head_ref TEXT NOT NULL,
  author_agent_id TEXT,
  status TEXT NOT NULL DEFAULT 'proposed', -- proposed | reviewing | approved | merged | abandoned
  risk_level TEXT,                          -- low | medium | high | critical
  confidence REAL,                          -- 0.0..1.0
  run_id TEXT,
  diff_artifact_key TEXT,
  summary_artifact_key TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (tenant_id, id)
);

CREATE INDEX IF NOT EXISTS idx_change_set_tenant_repo ON change_set(tenant_id, repo);
CREATE INDEX IF NOT EXISTS idx_change_set_tenant_status ON change_set(tenant_id, status);
CREATE INDEX IF NOT EXISTS idx_change_set_run_id ON change_set(tenant_id, run_id);
