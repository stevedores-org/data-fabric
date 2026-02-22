-- WS4: Policy & Governance engine support tables

CREATE TABLE IF NOT EXISTS policy_escalations (
    id TEXT PRIMARY KEY,
    decision_id TEXT NOT NULL,
    action TEXT NOT NULL,
    actor TEXT NOT NULL,
    resource TEXT,
    risk_level TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending', -- pending|approved|rejected|resolved
    context TEXT,                            -- JSON payload for HITL
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_policy_escalations_status ON policy_escalations(status, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_policy_escalations_decision ON policy_escalations(decision_id);

CREATE TABLE IF NOT EXISTS policy_rate_limit_counters (
    id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    action_class TEXT NOT NULL,
    window_start_epoch INTEGER NOT NULL,
    window_seconds INTEGER NOT NULL,
    count INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_policy_rate_actor ON policy_rate_limit_counters(actor, action_class, window_start_epoch DESC);
