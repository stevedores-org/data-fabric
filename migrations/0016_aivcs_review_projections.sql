-- AIVCS review-projection tables (issue #148, wave-1 slice 2).
--
-- Provenance events alone (e.g. `aivcs.review.opened`, `human.review.comment_added`)
-- are sufficient for audit but not for the AIVCS UI: rendering a review pane
-- needs durable comment threads, resolved state, and inline file anchors that
-- can be paginated and updated independently of the event log.
--
-- These projection tables are written from the event taxonomy (slice 1) by a
-- follower process — they intentionally carry no business logic of their own,
-- only shape. CRUD helpers and HTTP routes are added in later slices.
--
-- Multi-tenancy: every table carries `tenant_id TEXT NOT NULL DEFAULT ''` as
-- the leading PRIMARY KEY component, matching the convention introduced in
-- migrations 0005 and 0014. The empty-string default marks pre-existing /
-- unattributed rows (there are none on first deploy, but the column shape
-- is consistent with the rest of the schema so backfills work).
--
-- Idempotency: every CREATE uses IF NOT EXISTS so the migration can be safely
-- re-applied during dev resets and CI replays.

-- ── review_thread ────────────────────────────────────────────────
-- One thread per review conversation. `change_set_id` is nullable because
-- the first wave of AIVCS reviews may be attached to a Run only (the
-- change_set entity is itself a later projection — see issue #148 §5).
CREATE TABLE IF NOT EXISTS review_thread (
    tenant_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    review_id TEXT NOT NULL,
    change_set_id TEXT,
    status TEXT NOT NULL DEFAULT 'open',     -- open | resolved
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    resolved_at TEXT,
    PRIMARY KEY (tenant_id, id)
);
CREATE INDEX IF NOT EXISTS idx_review_thread_tenant_review
    ON review_thread(tenant_id, review_id);

-- ── review_comment ───────────────────────────────────────────────
-- One row per comment in a thread. `actor` follows the AIVCS event taxonomy:
-- 'human:<id>' or 'agent:<id>'. `parent_comment_id` enables reply chains
-- without a separate adjacency table.
CREATE TABLE IF NOT EXISTS review_comment (
    tenant_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    actor TEXT NOT NULL,                     -- 'human:<id>' or 'agent:<id>'
    body TEXT NOT NULL,
    parent_comment_id TEXT,                  -- for reply chains
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, id)
);
CREATE INDEX IF NOT EXISTS idx_review_comment_tenant_thread
    ON review_comment(tenant_id, thread_id, created_at);

-- ── review_thread_resolution ─────────────────────────────────────
-- Optional one-row-per-thread resolution record. Kept in a separate table
-- (rather than denormalised onto review_thread) so the resolution metadata
-- — actor, free-form note, taxonomy — has a stable identity for audit and
-- future relationship edges (causality).
CREATE TABLE IF NOT EXISTS review_thread_resolution (
    tenant_id TEXT NOT NULL DEFAULT '',
    thread_id TEXT NOT NULL,
    resolved_by TEXT NOT NULL,
    resolution TEXT NOT NULL,                -- 'fixed' | 'wont_fix' | 'duplicate' | 'discussion'
    note TEXT,
    resolved_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, thread_id)
);

-- ── file_anchor ──────────────────────────────────────────────────
-- An inline anchor pins a thread to a line range in a file on a specific
-- side of a diff. A thread can have zero or more anchors (e.g. a global
-- comment has none; a multi-file thread has many).
CREATE TABLE IF NOT EXISTS file_anchor (
    tenant_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    side TEXT NOT NULL DEFAULT 'right',      -- 'left' (base) | 'right' (head)
    PRIMARY KEY (tenant_id, id)
);
CREATE INDEX IF NOT EXISTS idx_file_anchor_tenant_thread
    ON file_anchor(tenant_id, thread_id);
