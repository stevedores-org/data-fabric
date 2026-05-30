-- Play Definitions: Versioned multi-agent workflow blueprints
CREATE TABLE IF NOT EXISTS play_definitions (
    name TEXT PRIMARY KEY,
    goal TEXT NOT NULL,
    tasks_json TEXT NOT NULL, -- JSON array of PlayTaskDefinition
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Seed: SRE Incident Play
INSERT INTO play_definitions (name, goal, tasks_json)
VALUES (
    'sre-incident',
    'Investigate, remediate, and verify a service incident automatically.',
    '[
        {
            "id": "investigate",
            "task_type": "sre",
            "priority": 100,
            "params": { "action": "root_cause_analysis" },
            "depends_on": []
        },
        {
            "id": "remediate",
            "task_type": "developer",
            "priority": 80,
            "params": { "action": "apply_fix" },
            "depends_on": ["investigate"]
        },
        {
            "id": "verify",
            "task_type": "sre",
            "priority": 90,
            "params": { "action": "canary_test" },
            "depends_on": ["remediate"]
        },
        {
            "id": "report",
            "task_type": "scribe",
            "priority": 50,
            "params": { "action": "incident_summary" },
            "depends_on": ["verify"]
        }
    ]'
) ON CONFLICT(name) DO UPDATE SET 
    goal = excluded.goal,
    tasks_json = excluded.tasks_json,
    updated_at = datetime('now');
