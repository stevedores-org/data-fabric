-- Silver layer: normalized entities for data-fabric
-- D1 schema migration (SQLite)

-- Tasks: MCP task loop (claim/ack, leases)
CREATE TABLE IF NOT EXISTS task (
  id TEXT PRIMARY KEY,
  run_id TEXT,
  kind TEXT NOT NULL,
  payload TEXT, -- JSON
  status TEXT NOT NULL DEFAULT 'pending', -- pending | claimed | completed | failed
  claimed_at INTEGER,
  claimed_by TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_task_run_id ON task(run_id);
CREATE INDEX IF NOT EXISTS idx_task_status ON task(status);
CREATE INDEX IF NOT EXISTS idx_task_claimed_at ON task(claimed_at);

-- Memory: session/RAG memory, keyed for retrieval
CREATE TABLE IF NOT EXISTS memory (
  id TEXT PRIMARY KEY,
  run_id TEXT,
  scope TEXT NOT NULL,
  key TEXT NOT NULL,
  content TEXT,
  artifact_key TEXT, -- optional ref to R2 artifact
  embedding_id TEXT, -- optional Vectorize ref
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(scope, key)
);

CREATE INDEX IF NOT EXISTS idx_memory_run_id ON memory(run_id);
CREATE INDEX IF NOT EXISTS idx_memory_scope ON memory(scope);

-- Run events: append-only provenance (bronze → silver)
CREATE TABLE IF NOT EXISTS run_event (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  event_type TEXT NOT NULL,
  actor TEXT NOT NULL,
  payload TEXT, -- JSON
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_run_event_run_id ON run_event(run_id);
CREATE INDEX IF NOT EXISTS idx_run_event_created_at ON run_event(created_at);

-- Artifact metadata: references to R2 objects (body in R2, metadata in D1)
CREATE TABLE IF NOT EXISTS artifact (
  id TEXT PRIMARY KEY,
  run_id TEXT,
  content_type TEXT,
  size INTEGER NOT NULL DEFAULT 0,
  r2_key TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_artifact_run_id ON artifact(run_id);
CREATE INDEX IF NOT EXISTS idx_artifact_r2_key ON artifact(r2_key);

-- Context packs: retrieval-ready assemblies (gold layer metadata)
CREATE TABLE IF NOT EXISTS context_pack (
  id TEXT PRIMARY KEY,
  run_id TEXT,
  scope TEXT NOT NULL,
  summary TEXT,
  entity_refs TEXT, -- JSON array of entity ids
  created_at INTEGER NOT NULL,
  expires_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_context_pack_run_id ON context_pack(run_id);
CREATE INDEX IF NOT EXISTS idx_context_pack_expires_at ON context_pack(expires_at);

-- Policy decisions: governance audit log
CREATE TABLE IF NOT EXISTS policy_decision (
  id TEXT PRIMARY KEY,
  action TEXT NOT NULL,
  actor TEXT NOT NULL,
  resource TEXT,
  decision TEXT NOT NULL, -- allow | deny
  reason TEXT,
  context TEXT, -- JSON
  created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_policy_decision_actor ON policy_decision(actor);
CREATE INDEX IF NOT EXISTS idx_policy_decision_created_at ON policy_decision(created_at);
