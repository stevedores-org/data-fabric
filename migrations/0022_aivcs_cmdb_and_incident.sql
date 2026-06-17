-- AIVCS operational concepts: a CMDB of monitored web properties/endpoints,
-- and a first-class `incident` projection.
--
-- Source of truth for agentic uptime monitoring (ai-agent-uptime-sentinel):
-- the sentinel reads ENABLED endpoints from the CMDB, and on a detected
-- outage / breakage / degradation upserts an `incident` here (deduplicated by
-- `dedup_key`) and links the derived human-facing GitHub issue back onto it.
-- sre-agent / ciso-agent query `incident` as the operational system of record.
--
-- HITL visibility: every CMDB row carries `source` ('seed' | 'human' | 'agent')
-- plus `updated_by` / `updated_at`, so human-vs-agent changes are auditable, and
-- the baseline inventory is seeded by PR-reviewed migrations (the Lornu overlay
-- uses the reserved >=9000 range). Read routes expose the full inventory.
--
-- Projection-only (no business logic); CRUD helpers + HTTP routes land in a
-- later slice, per the convention in 0016. Multi-tenant + idempotent, matching
-- migrations 0016 / 0021.

-- ── cmdb_property ────────────────────────────────────────────────
-- A monitored web property (configuration item), e.g. lornu.ai, aivcs.io.
CREATE TABLE IF NOT EXISTS cmdb_property (
    tenant_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    domain TEXT NOT NULL,
    name TEXT,
    brand_group TEXT,                            -- lornu | liteworks | stevedores | ...
    criticality TEXT NOT NULL DEFAULT 'normal',  -- critical | high | normal
    owner TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    source TEXT NOT NULL DEFAULT 'agent',         -- seed | human | agent  (HITL provenance)
    updated_by TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, id)
);
CREATE INDEX IF NOT EXISTS idx_cmdb_property_tenant_enabled
    ON cmdb_property(tenant_id, enabled);
CREATE UNIQUE INDEX IF NOT EXISTS idx_cmdb_property_tenant_domain
    ON cmdb_property(tenant_id, domain);

-- ── cmdb_endpoint ────────────────────────────────────────────────
-- A monitored endpoint under a property, e.g. https://dfc.aivcs.io/health.
-- A property has one or more endpoints; `latency_slo_ms` defines the
-- "performance downturn" threshold for degradation detection (NULL = none).
CREATE TABLE IF NOT EXISTS cmdb_endpoint (
    tenant_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    property_id TEXT NOT NULL,
    url TEXT NOT NULL,
    method TEXT NOT NULL DEFAULT 'GET',
    check_type TEXT NOT NULL DEFAULT 'http',      -- http | dns | tls
    expected_status INTEGER NOT NULL DEFAULT 200,
    latency_slo_ms INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    source TEXT NOT NULL DEFAULT 'agent',
    updated_by TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, id)
);
CREATE INDEX IF NOT EXISTS idx_cmdb_endpoint_tenant_enabled
    ON cmdb_endpoint(tenant_id, enabled);
CREATE INDEX IF NOT EXISTS idx_cmdb_endpoint_tenant_property
    ON cmdb_endpoint(tenant_id, property_id);

-- ── incident ─────────────────────────────────────────────────────
-- A first-class AIVCS operational incident. `dedup_key` (property:endpoint:type)
-- keeps a single OPEN incident per ongoing problem (enforced in app logic; the
-- index supports the open-incident lookup). The GitHub issue is a derived view
-- linked via `github_issue_*`; lifecycle is open -> ... -> resolved with MTTR.
CREATE TABLE IF NOT EXISTS incident (
    tenant_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    property_id TEXT,
    endpoint_id TEXT,
    dedup_key TEXT NOT NULL,
    type TEXT NOT NULL,                           -- outage | breakage | degradation
    severity TEXT NOT NULL DEFAULT 'sev3',        -- sev1 | sev2 | sev3
    status TEXT NOT NULL DEFAULT 'open',          -- open | acknowledged | mitigating | resolved
    signal TEXT,                                  -- what tripped (e.g. "503 x5", "p95 1240ms > 800ms")
    detector TEXT NOT NULL DEFAULT 'uptime-sentinel',
    github_issue_number INTEGER,
    github_issue_url TEXT,
    detected_at TEXT NOT NULL DEFAULT (datetime('now')),
    resolved_at TEXT,
    mttr_seconds INTEGER,
    PRIMARY KEY (tenant_id, id)
);
CREATE INDEX IF NOT EXISTS idx_incident_tenant_dedup_status
    ON incident(tenant_id, dedup_key, status);
CREATE INDEX IF NOT EXISTS idx_incident_tenant_status
    ON incident(tenant_id, status, detected_at);
