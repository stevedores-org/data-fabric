-- AIVCS review projections: index for pr_id-keyed reads (issue #159).
--
-- The AIVCS UI's `/pull-requests/:id/diff` and `/intent` routes
-- (stevedores-org/aivcs-api#10) read review threads and file anchors by
-- pull-request id. In data-fabric the native PR-equivalent entity is the
-- `change_set` (see 0015_aivcs_change_set.sql), and `review_thread.change_set_id`
-- is that linkage (0016_aivcs_review_projections.sql). So `pr_id == change_set_id`.
--
-- 0016 only indexed review_thread by (tenant_id, review_id); the pr_id reads
-- filter by (tenant_id, change_set_id), and the file-anchor read joins
-- file_anchor → review_thread on thread_id and filters the same way. This index
-- keeps both reads off a full table scan. IF NOT EXISTS for idempotent replays.
CREATE INDEX IF NOT EXISTS idx_review_thread_tenant_change_set
    ON review_thread(tenant_id, change_set_id);
