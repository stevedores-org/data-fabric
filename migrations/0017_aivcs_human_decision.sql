-- Issue #148 — AIVCS UI as a human-facing control plane.
-- Slice 3 (Human guidance loop) wave-1: human_decision projection.
--
-- The issue's "Human decision entity or projection" section calls for
-- storing human decisions as a typed projection alongside the immutable
-- events that record them. Events remain the source of truth in
-- events_bronze; this projection gives the UI / BFF a fast way to list
-- approvals, change-requests, pauses, resumes, and merges per run or
-- per review without scanning the event log.
--
-- Tenant scoping (WS8): tenant_id is the leading column of the composite
-- primary key and the leading column of every index, so every read path
-- can be tenant-bound from the index. Cross-tenant safety tests in
-- src/db.rs pin the SQL shape (tenant_id as ?1) for the helpers below.
--
-- Slice scope: projection-only. No HTTP route, no insert-from-event
-- trigger, no review/comment projection (slice 4), no guarded merge
-- (slice 4 in the issue numbering — separate slice), no change_set
-- table (separate slice).

CREATE TABLE IF NOT EXISTS human_decision (
  tenant_id TEXT NOT NULL DEFAULT '',
  id TEXT NOT NULL,
  run_id TEXT,
  review_id TEXT,
  actor TEXT NOT NULL,                              -- 'human:<id>'
  decision_type TEXT NOT NULL,                      -- approve | request_changes | pause | resume | merge
  reason TEXT,
  policy_decision_id TEXT,                          -- FK to policy_decisions if a policy check preceded
  resulting_event_id TEXT,                          -- FK to events_bronze id (the immutable record of the decision)
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (tenant_id, id)
);
CREATE INDEX IF NOT EXISTS idx_human_decision_tenant_run ON human_decision(tenant_id, run_id);
CREATE INDEX IF NOT EXISTS idx_human_decision_tenant_review ON human_decision(tenant_id, review_id);
CREATE INDEX IF NOT EXISTS idx_human_decision_tenant_type ON human_decision(tenant_id, decision_type);
