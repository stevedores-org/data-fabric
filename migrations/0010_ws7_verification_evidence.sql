-- WS7 Trust & Verification: queryable verification evidence ledger
CREATE TABLE IF NOT EXISTS verification_evidence (
    tenant_id TEXT NOT NULL,
    id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    baseline_run_id TEXT,
    status TEXT NOT NULL,
    step_count INTEGER NOT NULL,
    drift_count INTEGER NOT NULL,
    drift_ratio_percent REAL NOT NULL,
    within_variance INTEGER NOT NULL,
    failure_classification TEXT,
    tests_passed INTEGER NOT NULL,
    policy_approved INTEGER NOT NULL,
    provenance_complete INTEGER NOT NULL,
    eligible_for_promotion INTEGER NOT NULL,
    confidence_score INTEGER NOT NULL,
    failed_gates TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, id)
);

CREATE INDEX IF NOT EXISTS idx_verification_evidence_tenant_run_created
  ON verification_evidence(tenant_id, run_id, created_at DESC);
