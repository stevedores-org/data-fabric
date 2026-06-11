-- WS8 addendum: add tenant_id to policy_escalations, policy_rate_limit_counters,
-- and play_definitions.
--
-- These three tables were missed by 0005_ws8_multi_tenant.sql / 0012. Without
-- this migration, escalations, rate-limit counters, and named play definitions
-- are global across tenants — which is a cross-tenant data leak (escalations,
-- play definitions) and a noisy-neighbour vector (rate-limit counters).
--
-- SQLite / D1 do not allow ALTER TABLE ADD COLUMN with NOT NULL unless a
-- default is provided. We use an empty-string default for two reasons:
--   1. Pre-existing rows are deliberately marked "unattributed" (distinct
--      from the literal tenant 'default' used by other tables in 0005), so a
--      follow-up backfill job can target them by `WHERE tenant_id = ''`.
--   2. New writes from db.rs after this migration MUST supply a real
--      tenant_id; the worker code is updated in the same PR to do so.
--
-- A follow-up issue tracks backfilling pre-existing rows.

ALTER TABLE policy_escalations ADD COLUMN tenant_id TEXT NOT NULL DEFAULT '';
ALTER TABLE policy_rate_limit_counters ADD COLUMN tenant_id TEXT NOT NULL DEFAULT '';

-- Rebuild play_definitions to relax the PRIMARY KEY name constraint and
-- enforce composite PRIMARY KEY (tenant_id, name) for multi-tenancy.
ALTER TABLE play_definitions RENAME TO play_definitions_old;

CREATE TABLE play_definitions (
    tenant_id TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL,
    goal TEXT NOT NULL,
    tasks_json TEXT NOT NULL, -- JSON array of PlayTaskDefinition
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, name)
);

INSERT INTO play_definitions (tenant_id, name, goal, tasks_json, created_at, updated_at)
SELECT '', name, goal, tasks_json, created_at, updated_at FROM play_definitions_old;

DROP TABLE play_definitions_old;

-- Indexes for tenant-scoped queries.
CREATE INDEX IF NOT EXISTS idx_policy_escalations_tenant
    ON policy_escalations(tenant_id, status, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_policy_rate_limit_counters_tenant
    ON policy_rate_limit_counters(tenant_id, actor, action_class, window_start_epoch DESC);

-- The 'sre-incident' play was seeded by 0012 before tenant_id existed.
-- Re-attribute it to the literal tenant 'default' so that the platform-
-- default tenant continues to see the play. Other tenants must register
-- their own plays. Pre-existing custom plays remain at tenant_id = '' and
-- are unreachable from the API (which now requires a real tenant_id) —
-- they will be picked up by the follow-up backfill issue.
UPDATE play_definitions SET tenant_id = 'default' WHERE name = 'sre-incident';

-- Note on uniqueness for policy_rate_limit_counters:
--
-- 0004_ws4_policy_engine.sql defines `id` as the PRIMARY KEY, and db.rs
-- already constructs a synthetic counter id of the shape
--   "{actor}|{action_class}|{window_start}|{window_seconds}"
-- and relies on `ON CONFLICT(id) DO UPDATE` for atomic increment.
--
-- In this PR the counter id is extended to include tenant_id as the first
-- segment:
--   "{tenant_id}|{actor}|{action_class}|{window_start}|{window_seconds}"
-- which keeps the same PRIMARY KEY uniqueness contract (one row per
-- tenant + actor + action_class + window) without requiring a composite
-- UNIQUE constraint on top of the existing PK. Adding such a UNIQUE to an
-- existing SQLite table is non-trivial (table rebuild), so we route
-- uniqueness through the synthetic PK as the existing code already does.
--
-- Note on play_definitions PRIMARY KEY:
--
-- We relaxed the primary key of `play_definitions` by rebuilding the table.
-- The primary key is now `(tenant_id, name)` so that different tenants
-- can register plays with colliding names without interference or overwrites.
