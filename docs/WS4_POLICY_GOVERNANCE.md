# WS4: Policy and Governance

Implemented scope:

- `POST /v1/policies/check`:
  - risk taxonomy classification (`low|medium|high|critical`)
  - KV-backed policy bundle lookup with built-in fallback
  - explicit high-risk escalation when no matching allow rule exists
  - per-actor/action-class rate limiting
  - D1 decision persistence with context (`risk_level`, `policy_version`, `matched_rule`, `escalation_id`, `rate_limited`)
- `PUT /v1/policies/definitions/:version`:
  - validates/stores policy bundle (R2 source of truth)
  - mirrors to KV when `POLICY_KV` binding is present
  - optional activation
- `POST /v1/policies/activate/:version`:
  - updates active policy version in KV
- `GET /v1/policies/active`:
  - returns active version (`kv` or `builtin` source)
- `POST /v1/retention/run`:
  - TTL cleanup for `events_bronze`, `policy_decisions`, `checkpoints`, `artifacts`
  - deletes associated R2 objects for old checkpoints/artifacts

Schema added:

- `policy_escalations` (HITL queue)
- `policy_rate_limit_counters` (rate limiting)

Notes:

- If `POLICY_KV` is not configured, policy evaluation still works via built-in defaults.
- Built-in defaults intentionally escalate high-risk operations unless explicitly allowed by policy.
