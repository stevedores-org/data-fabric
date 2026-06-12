-- AIVCS slice 4 (issue #148, 'Pause Agent' command-wiring):
-- ensure runs has the columns needed for explicit pause/resume.
--
-- Background: the runs table already has a `status` column (see
-- migrations/0001_ws2_domain_model.sql), so this migration is mostly a
-- forward-compatibility guard. We:
--
--   1. Add `paused_at` / `resumed_at` timestamp columns (the pause/resume
--      handlers stamp these on transition, separately from `updated_at`
--      which any write touches).
--   2. Backfill any historical rows where `status` somehow ended up NULL
--      (shouldn't happen given the NOT NULL DEFAULT, but defensive).
--
-- Everything here uses `IF NOT EXISTS` / pre-checks so re-running is a no-op
-- and the migration runner stays idempotent.
--
-- NOTE: SQLite (D1) does not support `ALTER TABLE ... ADD COLUMN IF NOT
-- EXISTS`. We use the `CREATE TABLE IF NOT EXISTS` + index pattern for new
-- objects and rely on the migration runner's idempotency for ADD COLUMN.
-- If a future migration replays this and the column already exists, the
-- runner will see the "duplicate column" error and skip — same pattern as
-- migrations/0005_ws8_multi_tenant.sql.

ALTER TABLE runs ADD COLUMN paused_at TEXT;
ALTER TABLE runs ADD COLUMN resumed_at TEXT;

-- Backfill: existing rows without a status should be treated as 'running'
-- so the pause guard ("status NOT IN terminal-states") behaves correctly.
-- The DEFAULT 'created' from 0001 means this is a no-op on a clean DB —
-- it only matters for any legacy rows that pre-date the default.
UPDATE runs SET status = 'running' WHERE status IS NULL OR status = '';

-- Helpful index for the common pause/resume lookup pattern (tenant + run id
-- + status guard). Idempotent.
CREATE INDEX IF NOT EXISTS idx_runs_tenant_id_status
    ON runs(tenant_id, id, status);
