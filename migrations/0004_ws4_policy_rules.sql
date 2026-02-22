-- WS4: Policy rules engine — configurable action × resource × actor → verdict
-- Supports wildcard matching via '*' and specificity-ranked evaluation.

CREATE TABLE IF NOT EXISTS policy_rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    -- Match criteria (use '*' for wildcard)
    action_pattern TEXT NOT NULL DEFAULT '*',
    resource_pattern TEXT NOT NULL DEFAULT '*',
    actor_pattern TEXT NOT NULL DEFAULT '*',
    -- Risk classification: read, write, destructive, irreversible
    risk_level TEXT NOT NULL DEFAULT 'read',
    -- Verdict: allow, deny, escalate
    verdict TEXT NOT NULL DEFAULT 'deny',
    reason TEXT NOT NULL DEFAULT '',
    priority INTEGER NOT NULL DEFAULT 0,  -- higher = evaluated first
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_policy_rules_enabled ON policy_rules(enabled, priority DESC);
CREATE INDEX IF NOT EXISTS idx_policy_rules_action ON policy_rules(action_pattern);
