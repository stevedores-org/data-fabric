-- WS5: Add tenant_id to memory tables for multi-tenant isolation (#45)
-- All other subsystems already have tenant_id; these 4 tables were missed.

ALTER TABLE memory_index ADD COLUMN tenant_id TEXT NOT NULL DEFAULT '';
ALTER TABLE memory_retrieval_queries ADD COLUMN tenant_id TEXT NOT NULL DEFAULT '';
ALTER TABLE memory_retrieval_feedback ADD COLUMN tenant_id TEXT NOT NULL DEFAULT '';
ALTER TABLE memory ADD COLUMN tenant_id TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_memory_index_tenant ON memory_index(tenant_id);
CREATE INDEX IF NOT EXISTS idx_memory_queries_tenant ON memory_retrieval_queries(tenant_id);
