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
ALTER TABLE play_definitions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT '';

-- Indexes for tenant-scoped queries.
CREATE INDEX IF NOT EXISTS idx_policy_escalations_tenant
    ON policy_escalations(tenant_id, status, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_policy_rate_limit_counters_tenant
    ON policy_rate_limit_counters(tenant_id, actor, action_class, window_start_epoch DESC);
CREATE INDEX IF NOT EXISTS idx_play_definitions_tenant
    ON play_definitions(tenant_id, name);

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
-- 0012 declared `name TEXT PRIMARY KEY`. Strictly we now want
-- (tenant_id, name) as the primary key, but changing a SQLite PRIMARY KEY
-- requires a table rebuild. The application enforces the (tenant_id, name)
-- uniqueness on the write path via tenant-scoped UPSERT, and reads always
-- filter on tenant_id, so the existing PK remains correct (name remains
-- globally unique). A follow-up migration tracks the PK relaxation if
-- two tenants need to register plays with the same name.
