-- WS6: Orchestration Integration — integration registry
-- Tracks active integrations with external systems (oxidizedgraph, aivcs, llama.rs)

CREATE TABLE IF NOT EXISTS integrations (
  id TEXT PRIMARY KEY,
  target TEXT NOT NULL, -- oxidizedgraph | aivcs | llama_rs
  name TEXT NOT NULL,
  endpoint TEXT,
  api_version TEXT NOT NULL DEFAULT 'v1',
  status TEXT NOT NULL DEFAULT 'active', -- active | inactive | error
  config TEXT, -- JSON
  created_at TEXT NOT NULL,
  updated_at TEXT,
  last_seen_at TEXT,
  UNIQUE(target, name)
);

CREATE INDEX IF NOT EXISTS idx_integrations_target ON integrations(target);
CREATE INDEX IF NOT EXISTS idx_integrations_status ON integrations(status);
