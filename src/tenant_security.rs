//! WS8: Multi-Tenant Security — Isolation Without Fragmentation.
//!
//! Provides tenant isolation, authorization, rate limiting, secret handling,
//! and cross-tenant federation for the Cloudflare Workers data fabric.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;

// ── 1. Tenant Isolation ────────────────────────────────────────────

/// Tenant context extracted from a request, carrying identity, role,
/// permissions, and rate-limit configuration.
#[derive(Debug, Clone)]
pub struct TenantContext {
    pub tenant_id: String,
    pub role: Role,
    pub permissions: HashSet<Permission>,
    pub rate_limit_config: TenantRateLimitConfig,
}

impl TenantContext {
    /// Build a `TenantContext` from a tenant ID and role, deriving
    /// permissions from the default [`AuthzPolicy`].
    pub fn new(tenant_id: String, role: Role) -> Self {
        let permissions = AuthzPolicy::default().permissions_for_role(&role);
        Self {
            tenant_id,
            role,
            permissions,
            rate_limit_config: TenantRateLimitConfig::default(),
        }
    }

    /// Build with a custom rate-limit configuration.
    pub fn with_rate_limit(mut self, config: TenantRateLimitConfig) -> Self {
        self.rate_limit_config = config;
        self
    }
}

/// Resource descriptor used for access checks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Resource {
    /// The owning tenant ID embedded in the resource.
    pub tenant_id: String,
    /// Resource type (e.g. "run", "artifact", "checkpoint").
    pub resource_type: String,
    /// Resource identifier.
    pub resource_id: String,
}

/// Validate that `ctx` owns `resource` (tenant boundary check).
pub fn validate_tenant_access(ctx: &TenantContext, resource: &Resource) -> Result<(), AuthzError> {
    if ctx.tenant_id == resource.tenant_id {
        Ok(())
    } else {
        Err(AuthzError::CrossTenantAccess {
            requesting_tenant: ctx.tenant_id.clone(),
            resource_tenant: resource.tenant_id.clone(),
        })
    }
}

/// Generate an R2 key with tenant prefix for blob storage isolation.
///
/// Format: `tenants/{tenant_id}/{resource_type}/{id}`
pub fn partition_key(tenant_id: &str, resource_type: &str, id: &str) -> String {
    format!("tenants/{tenant_id}/{resource_type}/{id}")
}

/// Return a SQL WHERE clause fragment for D1 row-level security.
///
/// The caller must bind `tenant_id` as a parameter at the returned
/// placeholder position.
pub fn tenant_query_filter(tenant_id: &str) -> TenantFilter {
    TenantFilter {
        clause: "tenant_id = ?".to_string(),
        value: tenant_id.to_string(),
    }
}

/// A SQL filter fragment with its bind value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantFilter {
    pub clause: String,
    pub value: String,
}

// ── 2. AuthZ Framework ─────────────────────────────────────────────

/// Actions that can be performed on resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Read,
    Write,
    Admin,
    Federation,
}

/// Roles assignable to tenant members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Reader,
    Contributor,
    Admin,
    SystemService,
}

/// Maps roles to allowed permissions per resource type.
///
/// The default policy applies broadly (resource type `"*"`).
/// More specific resource-type entries override the wildcard.
#[derive(Debug, Clone)]
pub struct AuthzPolicy {
    /// (role, resource_type) -> set of permissions.
    rules: HashMap<(Role, String), HashSet<Permission>>,
}

impl Default for AuthzPolicy {
    fn default() -> Self {
        let mut rules: HashMap<(Role, String), HashSet<Permission>> = HashMap::new();

        // Reader: read-only everywhere.
        rules.insert(
            (Role::Reader, "*".into()),
            [Permission::Read].into_iter().collect(),
        );

        // Contributor: read + write.
        rules.insert(
            (Role::Contributor, "*".into()),
            [Permission::Read, Permission::Write].into_iter().collect(),
        );

        // Admin: full access.
        rules.insert(
            (Role::Admin, "*".into()),
            [
                Permission::Read,
                Permission::Write,
                Permission::Admin,
                Permission::Federation,
            ]
            .into_iter()
            .collect(),
        );

        // SystemService: read + write + federation (no Admin UI).
        rules.insert(
            (Role::SystemService, "*".into()),
            [Permission::Read, Permission::Write, Permission::Federation]
                .into_iter()
                .collect(),
        );

        Self { rules }
    }
}

impl AuthzPolicy {
    /// Resolve the effective permission set for a role, optionally scoped
    /// to a resource type.  Falls back to the wildcard entry.
    pub fn permissions_for_role(&self, role: &Role) -> HashSet<Permission> {
        self.rules
            .get(&(*role, "*".into()))
            .cloned()
            .unwrap_or_default()
    }

    /// Resolve permissions for a role scoped to a specific resource type.
    pub fn permissions_for_role_resource(
        &self,
        role: &Role,
        resource_type: &str,
    ) -> HashSet<Permission> {
        self.rules
            .get(&(*role, resource_type.to_string()))
            .or_else(|| self.rules.get(&(*role, "*".into())))
            .cloned()
            .unwrap_or_default()
    }
}

/// Token claims extracted from request authentication.
#[derive(Debug, Clone)]
pub struct TokenClaims {
    pub tenant_id: String,
    pub role: Role,
    /// Optional explicit permission overrides from the token.
    pub scoped_permissions: Option<HashSet<Permission>>,
}

/// Errors produced by the authorization framework.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthzError {
    /// The action is not permitted for this role.
    PermissionDenied {
        role: Role,
        required: Permission,
    },
    /// Attempted cross-tenant resource access.
    CrossTenantAccess {
        requesting_tenant: String,
        resource_tenant: String,
    },
    /// Rate limit exceeded.
    RateLimitExceeded {
        tenant_id: String,
        limit: u64,
    },
}

impl fmt::Display for AuthzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthzError::PermissionDenied { role, required } => {
                write!(f, "role {role:?} lacks {required:?} permission")
            }
            AuthzError::CrossTenantAccess {
                requesting_tenant,
                resource_tenant,
            } => {
                write!(
                    f,
                    "tenant {requesting_tenant} cannot access resources of tenant {resource_tenant}"
                )
            }
            AuthzError::RateLimitExceeded { tenant_id, limit } => {
                write!(f, "tenant {tenant_id} exceeded rate limit ({limit} rpm)")
            }
        }
    }
}

/// Evaluate authorization: pure in-memory, no I/O.  Target: <1ms.
///
/// 1. Checks tenant boundary (token tenant must match resource tenant).
/// 2. Checks role-permission matrix for the requested action.
pub fn evaluate_authz(
    claims: &TokenClaims,
    resource: &Resource,
    action: Permission,
) -> Result<(), AuthzError> {
    // 1. Tenant boundary.
    if claims.tenant_id != resource.tenant_id {
        return Err(AuthzError::CrossTenantAccess {
            requesting_tenant: claims.tenant_id.clone(),
            resource_tenant: resource.tenant_id.clone(),
        });
    }

    // 2. Permission check — prefer token-scoped overrides, else policy.
    let effective = match &claims.scoped_permissions {
        Some(scoped) => scoped.clone(),
        None => AuthzPolicy::default().permissions_for_role_resource(
            &claims.role,
            &resource.resource_type,
        ),
    };

    if effective.contains(&action) {
        Ok(())
    } else {
        Err(AuthzError::PermissionDenied {
            role: claims.role,
            required: action,
        })
    }
}

// ── 3. Rate Limiting ───────────────────────────────────────────────

/// Per-tenant rate-limit configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantRateLimitConfig {
    pub requests_per_minute: u64,
    pub burst_limit: u64,
    pub quota_bytes: u64,
}

impl Default for TenantRateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 120,
            burst_limit: 20,
            quota_bytes: 5 * 1024 * 1024 * 1024, // 5 GiB
        }
    }
}

/// Sliding-window counter kept in Worker memory for the Worker lifetime.
#[derive(Debug, Clone)]
pub struct RateLimitState {
    /// Timestamps (epoch millis) of requests within the current window.
    window_hits: Vec<u64>,
    /// Consecutive failure count for circuit breaker.
    consecutive_failures: u32,
    /// If set, circuit is open until this epoch-ms timestamp.
    circuit_open_until: Option<u64>,
}

impl RateLimitState {
    pub fn new() -> Self {
        Self {
            window_hits: Vec::new(),
            consecutive_failures: 0,
            circuit_open_until: None,
        }
    }

    /// Record a successful request (resets failure counter).
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Record a failed request (increments failure counter).
    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
    }

    /// Number of consecutive failures.
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Number of requests in the current window.
    pub fn window_count(&self) -> usize {
        self.window_hits.len()
    }
}

/// Errors from rate limiting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitError {
    /// Request rate exceeded.
    Exceeded { tenant_id: String, limit: u64 },
    /// Circuit breaker is open — short-circuiting.
    CircuitOpen { tenant_id: String, until_ms: u64 },
}

impl fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RateLimitError::Exceeded { tenant_id, limit } => {
                write!(f, "tenant {tenant_id}: rate limit exceeded ({limit} rpm)")
            }
            RateLimitError::CircuitOpen {
                tenant_id,
                until_ms,
            } => {
                write!(
                    f,
                    "tenant {tenant_id}: circuit open until epoch ms {until_ms}"
                )
            }
        }
    }
}

/// Circuit breaker threshold: after this many consecutive failures,
/// the circuit opens for `CIRCUIT_COOLDOWN_MS`.
const CIRCUIT_BREAKER_THRESHOLD: u32 = 5;
/// Cooldown period when circuit breaker trips (30 seconds).
const CIRCUIT_COOLDOWN_MS: u64 = 30_000;

/// Check rate limit using a sliding window.
///
/// `now_ms` is the current epoch-millisecond timestamp (caller provides
/// for testability and WASM compatibility — avoids `std::time::Instant`
/// in non-test code).
pub fn check_rate_limit(
    tenant_id: &str,
    config: &TenantRateLimitConfig,
    state: &mut RateLimitState,
    now_ms: u64,
) -> Result<(), RateLimitError> {
    // Circuit breaker check.
    if let Some(until) = state.circuit_open_until {
        if now_ms < until {
            return Err(RateLimitError::CircuitOpen {
                tenant_id: tenant_id.to_string(),
                until_ms: until,
            });
        }
        // Cooldown elapsed — close circuit.
        state.circuit_open_until = None;
        state.consecutive_failures = 0;
    }

    // Trip circuit breaker if too many consecutive failures.
    if state.consecutive_failures >= CIRCUIT_BREAKER_THRESHOLD {
        let until = now_ms + CIRCUIT_COOLDOWN_MS;
        state.circuit_open_until = Some(until);
        return Err(RateLimitError::CircuitOpen {
            tenant_id: tenant_id.to_string(),
            until_ms: until,
        });
    }

    // Sliding window: remove entries older than 60 seconds.
    let window_start = now_ms.saturating_sub(60_000);
    state.window_hits.retain(|&ts| ts >= window_start);

    // Check limit (allow burst up to burst_limit above sustained rate).
    let effective_limit = config.requests_per_minute + config.burst_limit;
    if state.window_hits.len() as u64 >= effective_limit {
        return Err(RateLimitError::Exceeded {
            tenant_id: tenant_id.to_string(),
            limit: config.requests_per_minute,
        });
    }

    // Record this request.
    state.window_hits.push(now_ms);
    Ok(())
}

// ── 4. Secret Handling ─────────────────────────────────────────────

/// Classification for data sensitivity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretClassification {
    Public,
    Internal,
    Confidential,
    Restricted,
}

/// Known patterns for field names that indicate secrets.
const RESTRICTED_FIELDS: &[&str] = &[
    "password",
    "secret",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "credential",
    "credentials",
    "ssn",
    "credit_card",
];

const CONFIDENTIAL_FIELDS: &[&str] = &[
    "email",
    "phone",
    "address",
    "ip_address",
    "session_id",
    "cookie",
];

const INTERNAL_FIELDS: &[&str] = &[
    "tenant_id",
    "user_id",
    "account_id",
    "internal_id",
    "trace_id",
];

/// Classify a field name based on naming patterns.
pub fn classify_field(field_name: &str) -> SecretClassification {
    let lower = field_name.to_ascii_lowercase();
    if RESTRICTED_FIELDS.iter().any(|p| lower.contains(p)) {
        SecretClassification::Restricted
    } else if CONFIDENTIAL_FIELDS.iter().any(|p| lower.contains(p)) {
        SecretClassification::Confidential
    } else if INTERNAL_FIELDS.iter().any(|p| lower.contains(p)) {
        SecretClassification::Internal
    } else {
        SecretClassification::Public
    }
}

/// Validate that a JSON record contains no plaintext secrets.
///
/// Returns `Ok(())` if all fields classified as Restricted or Confidential
/// have placeholder/encrypted values (i.e., start with `"enc:"` or `"***"`).
/// Otherwise returns `Err` with the offending field names.
pub fn validate_no_plaintext_secrets(
    record: &serde_json::Value,
) -> Result<(), Vec<String>> {
    let mut violations = Vec::new();

    if let Some(obj) = record.as_object() {
        for (key, value) in obj {
            let classification = classify_field(key);
            if classification >= SecretClassification::Confidential {
                if let Some(s) = value.as_str() {
                    if !s.starts_with("enc:") && !s.starts_with("***") && !s.is_empty() {
                        violations.push(key.clone());
                    }
                }
            }
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Redact sensitive fields for logging/audit.
///
/// Returns a copy with Restricted fields set to `"***REDACTED***"`,
/// and Confidential fields partially masked.
pub fn redact_sensitive_fields(record: &serde_json::Value) -> serde_json::Value {
    match record {
        serde_json::Value::Object(obj) => {
            let mut out = serde_json::Map::new();
            for (key, value) in obj {
                let classification = classify_field(key);
                let redacted = match classification {
                    SecretClassification::Restricted => {
                        serde_json::Value::String("***REDACTED***".into())
                    }
                    SecretClassification::Confidential => {
                        if let Some(s) = value.as_str() {
                            if s.len() > 4 {
                                serde_json::Value::String(format!("{}***", &s[..2]))
                            } else {
                                serde_json::Value::String("***".into())
                            }
                        } else {
                            serde_json::Value::String("***".into())
                        }
                    }
                    _ => value.clone(),
                };
                out.insert(key.clone(), redacted);
            }
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    }
}

// ── 5. Cross-Tenant Federation ─────────────────────────────────────

/// Configuration for cross-tenant data sharing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct FederationConfig {
    /// Whether this tenant has opted into federation.
    pub opt_in: bool,
    /// Tenant IDs allowed to receive federated data.
    pub allowed_tenants: HashSet<String>,
    /// Types of data that may be shared.
    pub sharing_scope: HashSet<String>,
}


/// Check whether federation is allowed between two tenants for a given
/// data type.
pub fn can_federate(
    source_config: &FederationConfig,
    target_tenant: &str,
    data_type: &str,
) -> bool {
    source_config.opt_in
        && source_config.allowed_tenants.contains(target_tenant)
        && source_config.sharing_scope.contains(data_type)
}

/// Strip PII and keep only aggregate-safe fields for federation.
///
/// Removes fields classified Confidential or Restricted and any field
/// named with common PII patterns.
pub fn anonymize_for_federation(data: &serde_json::Value) -> serde_json::Value {
    match data {
        serde_json::Value::Object(obj) => {
            let mut out = serde_json::Map::new();
            for (key, value) in obj {
                let classification = classify_field(key);
                if classification < SecretClassification::Confidential {
                    out.insert(key.clone(), anonymize_for_federation(value));
                }
                // Confidential and Restricted fields are stripped entirely.
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(anonymize_for_federation).collect())
        }
        other => other.clone(),
    }
}

// ── 6. Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Tenant isolation ───────────────────────────────────────

    #[test]
    fn validate_tenant_access_same_tenant_succeeds() {
        let ctx = TenantContext::new("tenant-a".into(), Role::Contributor);
        let res = Resource {
            tenant_id: "tenant-a".into(),
            resource_type: "run".into(),
            resource_id: "run-1".into(),
        };
        assert!(validate_tenant_access(&ctx, &res).is_ok());
    }

    #[test]
    fn validate_tenant_access_cross_tenant_denied() {
        let ctx = TenantContext::new("tenant-a".into(), Role::Admin);
        let res = Resource {
            tenant_id: "tenant-b".into(),
            resource_type: "run".into(),
            resource_id: "run-1".into(),
        };
        let err = validate_tenant_access(&ctx, &res).unwrap_err();
        assert!(matches!(err, AuthzError::CrossTenantAccess { .. }));
    }

    #[test]
    fn partition_key_format() {
        assert_eq!(
            partition_key("t1", "artifact", "abc"),
            "tenants/t1/artifact/abc"
        );
    }

    #[test]
    fn partition_key_different_resource_types() {
        assert_eq!(
            partition_key("org-x", "checkpoint", "cp-99"),
            "tenants/org-x/checkpoint/cp-99"
        );
    }

    #[test]
    fn tenant_query_filter_produces_valid_clause() {
        let f = tenant_query_filter("t42");
        assert_eq!(f.clause, "tenant_id = ?");
        assert_eq!(f.value, "t42");
    }

    // ── AuthZ: role-permission matrix ──────────────────────────

    #[test]
    fn reader_can_read() {
        let claims = TokenClaims {
            tenant_id: "t1".into(),
            role: Role::Reader,
            scoped_permissions: None,
        };
        let res = Resource {
            tenant_id: "t1".into(),
            resource_type: "run".into(),
            resource_id: "r1".into(),
        };
        assert!(evaluate_authz(&claims, &res, Permission::Read).is_ok());
    }

    #[test]
    fn reader_cannot_write() {
        let claims = TokenClaims {
            tenant_id: "t1".into(),
            role: Role::Reader,
            scoped_permissions: None,
        };
        let res = Resource {
            tenant_id: "t1".into(),
            resource_type: "run".into(),
            resource_id: "r1".into(),
        };
        assert!(evaluate_authz(&claims, &res, Permission::Write).is_err());
    }

    #[test]
    fn contributor_can_read_and_write() {
        let claims = TokenClaims {
            tenant_id: "t1".into(),
            role: Role::Contributor,
            scoped_permissions: None,
        };
        let res = Resource {
            tenant_id: "t1".into(),
            resource_type: "artifact".into(),
            resource_id: "a1".into(),
        };
        assert!(evaluate_authz(&claims, &res, Permission::Read).is_ok());
        assert!(evaluate_authz(&claims, &res, Permission::Write).is_ok());
    }

    #[test]
    fn contributor_cannot_admin() {
        let claims = TokenClaims {
            tenant_id: "t1".into(),
            role: Role::Contributor,
            scoped_permissions: None,
        };
        let res = Resource {
            tenant_id: "t1".into(),
            resource_type: "run".into(),
            resource_id: "r1".into(),
        };
        assert!(evaluate_authz(&claims, &res, Permission::Admin).is_err());
    }

    #[test]
    fn admin_has_all_permissions() {
        let claims = TokenClaims {
            tenant_id: "t1".into(),
            role: Role::Admin,
            scoped_permissions: None,
        };
        let res = Resource {
            tenant_id: "t1".into(),
            resource_type: "run".into(),
            resource_id: "r1".into(),
        };
        assert!(evaluate_authz(&claims, &res, Permission::Read).is_ok());
        assert!(evaluate_authz(&claims, &res, Permission::Write).is_ok());
        assert!(evaluate_authz(&claims, &res, Permission::Admin).is_ok());
        assert!(evaluate_authz(&claims, &res, Permission::Federation).is_ok());
    }

    #[test]
    fn system_service_has_federation_but_not_admin() {
        let claims = TokenClaims {
            tenant_id: "t1".into(),
            role: Role::SystemService,
            scoped_permissions: None,
        };
        let res = Resource {
            tenant_id: "t1".into(),
            resource_type: "run".into(),
            resource_id: "r1".into(),
        };
        assert!(evaluate_authz(&claims, &res, Permission::Federation).is_ok());
        assert!(evaluate_authz(&claims, &res, Permission::Admin).is_err());
    }

    #[test]
    fn cross_tenant_authz_denied() {
        let claims = TokenClaims {
            tenant_id: "t1".into(),
            role: Role::Admin,
            scoped_permissions: None,
        };
        let res = Resource {
            tenant_id: "t2".into(),
            resource_type: "run".into(),
            resource_id: "r1".into(),
        };
        let err = evaluate_authz(&claims, &res, Permission::Read).unwrap_err();
        assert!(matches!(err, AuthzError::CrossTenantAccess { .. }));
    }

    #[test]
    fn scoped_permissions_override_role_defaults() {
        // Reader role, but token grants Write explicitly.
        let claims = TokenClaims {
            tenant_id: "t1".into(),
            role: Role::Reader,
            scoped_permissions: Some([Permission::Write].into_iter().collect()),
        };
        let res = Resource {
            tenant_id: "t1".into(),
            resource_type: "run".into(),
            resource_id: "r1".into(),
        };
        // Write allowed via scope override.
        assert!(evaluate_authz(&claims, &res, Permission::Write).is_ok());
        // Read NOT in scope override — denied.
        assert!(evaluate_authz(&claims, &res, Permission::Read).is_err());
    }

    #[test]
    fn authz_error_display_messages() {
        let e1 = AuthzError::PermissionDenied {
            role: Role::Reader,
            required: Permission::Write,
        };
        assert!(e1.to_string().contains("Reader"));

        let e2 = AuthzError::CrossTenantAccess {
            requesting_tenant: "a".into(),
            resource_tenant: "b".into(),
        };
        assert!(e2.to_string().contains("cannot access"));
    }

    // ── AuthZ performance ──────────────────────────────────────

    #[test]
    fn authz_check_under_1ms() {
        use std::time::Instant;

        let claims = TokenClaims {
            tenant_id: "perf-tenant".into(),
            role: Role::Contributor,
            scoped_permissions: None,
        };
        let res = Resource {
            tenant_id: "perf-tenant".into(),
            resource_type: "artifact".into(),
            resource_id: "art-1".into(),
        };

        // Warm up.
        for _ in 0..100 {
            let _ = evaluate_authz(&claims, &res, Permission::Read);
        }

        let iterations = 10_000;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = evaluate_authz(&claims, &res, Permission::Read);
        }
        let elapsed = start.elapsed();
        let per_call_ns = elapsed.as_nanos() / iterations as u128;

        // p50 must be <1ms = 1_000_000ns.  With 10k iterations the average
        // is a solid proxy; in practice each call is <1us.
        assert!(
            per_call_ns < 1_000_000,
            "authz p50 exceeded 1ms: {}ns per call",
            per_call_ns
        );
    }

    // ── Rate limiting ──────────────────────────────────────────

    #[test]
    fn rate_limit_allows_under_limit() {
        let config = TenantRateLimitConfig {
            requests_per_minute: 10,
            burst_limit: 5,
            quota_bytes: 1024,
        };
        let mut state = RateLimitState::new();
        for i in 0..15 {
            assert!(
                check_rate_limit("t1", &config, &mut state, 1000 + i).is_ok(),
                "request {i} should be allowed"
            );
        }
        // 16th should be rejected (10 + 5 = 15 effective limit).
        assert!(check_rate_limit("t1", &config, &mut state, 1015).is_err());
    }

    #[test]
    fn rate_limit_window_slides() {
        let config = TenantRateLimitConfig {
            requests_per_minute: 2,
            burst_limit: 0,
            quota_bytes: 1024,
        };
        let mut state = RateLimitState::new();
        let base = 100_000u64;

        // Fill up.
        assert!(check_rate_limit("t1", &config, &mut state, base).is_ok());
        assert!(check_rate_limit("t1", &config, &mut state, base + 1).is_ok());
        assert!(check_rate_limit("t1", &config, &mut state, base + 2).is_err());

        // Advance past 60s window — old entries expire.
        assert!(check_rate_limit("t1", &config, &mut state, base + 61_000).is_ok());
    }

    #[test]
    fn circuit_breaker_trips_after_threshold() {
        let config = TenantRateLimitConfig::default();
        let mut state = RateLimitState::new();
        let now = 200_000u64;

        // Simulate consecutive failures.
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            state.record_failure();
        }

        let err = check_rate_limit("t1", &config, &mut state, now).unwrap_err();
        match err {
            RateLimitError::CircuitOpen { until_ms, .. } => {
                assert_eq!(until_ms, now + CIRCUIT_COOLDOWN_MS);
            }
            _ => panic!("expected CircuitOpen"),
        }
    }

    #[test]
    fn circuit_breaker_resets_after_cooldown() {
        let config = TenantRateLimitConfig::default();
        let mut state = RateLimitState::new();
        let now = 200_000u64;

        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            state.record_failure();
        }
        // Trip the breaker.
        let _ = check_rate_limit("t1", &config, &mut state, now);

        // After cooldown, should be allowed.
        let after_cooldown = now + CIRCUIT_COOLDOWN_MS + 1;
        assert!(check_rate_limit("t1", &config, &mut state, after_cooldown).is_ok());
        assert_eq!(state.consecutive_failures(), 0);
    }

    #[test]
    fn circuit_breaker_blocked_during_cooldown() {
        let config = TenantRateLimitConfig::default();
        let mut state = RateLimitState::new();
        let now = 200_000u64;

        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            state.record_failure();
        }
        let _ = check_rate_limit("t1", &config, &mut state, now);

        // Still within cooldown.
        let during_cooldown = now + CIRCUIT_COOLDOWN_MS - 1;
        assert!(check_rate_limit("t1", &config, &mut state, during_cooldown).is_err());
    }

    #[test]
    fn success_resets_failure_counter() {
        let mut state = RateLimitState::new();
        state.record_failure();
        state.record_failure();
        assert_eq!(state.consecutive_failures(), 2);
        state.record_success();
        assert_eq!(state.consecutive_failures(), 0);
    }

    // ── Secret handling ────────────────────────────────────────

    #[test]
    fn classify_field_restricted() {
        assert_eq!(classify_field("api_key"), SecretClassification::Restricted);
        assert_eq!(classify_field("password"), SecretClassification::Restricted);
        assert_eq!(
            classify_field("my_secret_value"),
            SecretClassification::Restricted
        );
        assert_eq!(
            classify_field("AUTH_TOKEN"),
            SecretClassification::Restricted
        );
        assert_eq!(
            classify_field("private_key"),
            SecretClassification::Restricted
        );
    }

    #[test]
    fn classify_field_confidential() {
        assert_eq!(classify_field("email"), SecretClassification::Confidential);
        assert_eq!(classify_field("phone"), SecretClassification::Confidential);
        assert_eq!(
            classify_field("ip_address"),
            SecretClassification::Confidential
        );
    }

    #[test]
    fn classify_field_internal() {
        assert_eq!(classify_field("tenant_id"), SecretClassification::Internal);
        assert_eq!(classify_field("user_id"), SecretClassification::Internal);
    }

    #[test]
    fn classify_field_public() {
        assert_eq!(classify_field("name"), SecretClassification::Public);
        assert_eq!(classify_field("status"), SecretClassification::Public);
        assert_eq!(classify_field("count"), SecretClassification::Public);
    }

    #[test]
    fn validate_no_plaintext_secrets_passes_for_clean_record() {
        let record = json!({
            "name": "test",
            "status": "active",
            "api_key": "enc:abc123",
            "email": "***masked"
        });
        assert!(validate_no_plaintext_secrets(&record).is_ok());
    }

    #[test]
    fn validate_no_plaintext_secrets_catches_violations() {
        let record = json!({
            "name": "test",
            "password": "hunter2",
            "email": "user@example.com"
        });
        let violations = validate_no_plaintext_secrets(&record).unwrap_err();
        assert!(violations.contains(&"password".to_string()));
        assert!(violations.contains(&"email".to_string()));
    }

    #[test]
    fn validate_no_plaintext_allows_empty_secrets() {
        let record = json!({
            "password": "",
            "email": ""
        });
        assert!(validate_no_plaintext_secrets(&record).is_ok());
    }

    #[test]
    fn validate_no_plaintext_allows_encrypted_prefix() {
        let record = json!({
            "secret": "enc:cipher-text-here",
            "token": "***"
        });
        assert!(validate_no_plaintext_secrets(&record).is_ok());
    }

    #[test]
    fn redact_sensitive_fields_redacts_restricted() {
        let record = json!({
            "name": "test",
            "api_key": "sk-abc123",
            "password": "hunter2"
        });
        let redacted = redact_sensitive_fields(&record);
        assert_eq!(redacted["name"], "test");
        assert_eq!(redacted["api_key"], "***REDACTED***");
        assert_eq!(redacted["password"], "***REDACTED***");
    }

    #[test]
    fn redact_sensitive_fields_masks_confidential() {
        let record = json!({
            "email": "user@example.com",
            "phone": "555-1234",
            "status": "active"
        });
        let redacted = redact_sensitive_fields(&record);
        assert_eq!(redacted["email"], "us***");
        assert_eq!(redacted["phone"], "55***");
        assert_eq!(redacted["status"], "active");
    }

    #[test]
    fn redact_sensitive_fields_short_confidential_value() {
        let record = json!({
            "email": "ab"
        });
        let redacted = redact_sensitive_fields(&record);
        assert_eq!(redacted["email"], "***");
    }

    #[test]
    fn redact_non_object_returns_clone() {
        let val = json!("just a string");
        assert_eq!(redact_sensitive_fields(&val), val);
    }

    // ── Federation ─────────────────────────────────────────────

    #[test]
    fn can_federate_when_opted_in_and_allowed() {
        let config = FederationConfig {
            opt_in: true,
            allowed_tenants: ["tenant-b".to_string()].into_iter().collect(),
            sharing_scope: ["run_summary".to_string()].into_iter().collect(),
        };
        assert!(can_federate(&config, "tenant-b", "run_summary"));
    }

    #[test]
    fn cannot_federate_when_opt_in_false() {
        let config = FederationConfig {
            opt_in: false,
            allowed_tenants: ["tenant-b".to_string()].into_iter().collect(),
            sharing_scope: ["run_summary".to_string()].into_iter().collect(),
        };
        assert!(!can_federate(&config, "tenant-b", "run_summary"));
    }

    #[test]
    fn cannot_federate_to_unallowed_tenant() {
        let config = FederationConfig {
            opt_in: true,
            allowed_tenants: ["tenant-b".to_string()].into_iter().collect(),
            sharing_scope: ["run_summary".to_string()].into_iter().collect(),
        };
        assert!(!can_federate(&config, "tenant-c", "run_summary"));
    }

    #[test]
    fn cannot_federate_out_of_scope_data_type() {
        let config = FederationConfig {
            opt_in: true,
            allowed_tenants: ["tenant-b".to_string()].into_iter().collect(),
            sharing_scope: ["run_summary".to_string()].into_iter().collect(),
        };
        assert!(!can_federate(&config, "tenant-b", "raw_events"));
    }

    #[test]
    fn anonymize_strips_sensitive_fields() {
        let data = json!({
            "run_id": "r-1",
            "email": "user@test.com",
            "password": "secret",
            "status": "ok",
            "count": 42
        });
        let anon = anonymize_for_federation(&data);
        let obj = anon.as_object().unwrap();

        // Public and Internal fields are kept.
        assert!(obj.contains_key("run_id"));
        assert!(obj.contains_key("status"));
        assert!(obj.contains_key("count"));

        // Confidential and Restricted fields are stripped.
        assert!(!obj.contains_key("email"));
        assert!(!obj.contains_key("password"));
    }

    #[test]
    fn anonymize_recurses_into_arrays() {
        let data = json!({
            "items": [
                {"name": "ok", "token": "secret123"},
                {"name": "also-ok", "email": "a@b.com"}
            ]
        });
        let anon = anonymize_for_federation(&data);
        let items = anon["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].as_object().unwrap().contains_key("name"));
        assert!(!items[0].as_object().unwrap().contains_key("token"));
        assert!(!items[1].as_object().unwrap().contains_key("email"));
    }

    #[test]
    fn anonymize_non_object_returns_clone() {
        assert_eq!(anonymize_for_federation(&json!(42)), json!(42));
        assert_eq!(anonymize_for_federation(&json!("hello")), json!("hello"));
    }

    #[test]
    fn default_federation_config_opts_out() {
        let config = FederationConfig::default();
        assert!(!config.opt_in);
        assert!(config.allowed_tenants.is_empty());
        assert!(config.sharing_scope.is_empty());
    }

    // ── Integration: full request flow ─────────────────────────

    #[test]
    fn full_flow_tenant_isolation_then_authz() {
        let ctx = TenantContext::new("acme".into(), Role::Contributor);
        let resource = Resource {
            tenant_id: "acme".into(),
            resource_type: "artifact".into(),
            resource_id: "a-1".into(),
        };

        // Step 1: tenant boundary check.
        assert!(validate_tenant_access(&ctx, &resource).is_ok());

        // Step 2: authz check.
        let claims = TokenClaims {
            tenant_id: "acme".into(),
            role: Role::Contributor,
            scoped_permissions: None,
        };
        assert!(evaluate_authz(&claims, &resource, Permission::Write).is_ok());

        // Step 3: generate isolated storage key.
        let key = partition_key("acme", "artifact", "a-1");
        assert!(key.starts_with("tenants/acme/"));
    }

    #[test]
    fn full_flow_cross_tenant_blocked_at_every_layer() {
        let ctx = TenantContext::new("acme".into(), Role::Admin);
        let foreign_resource = Resource {
            tenant_id: "evil-corp".into(),
            resource_type: "run".into(),
            resource_id: "r-1".into(),
        };

        // Isolation layer blocks it.
        assert!(validate_tenant_access(&ctx, &foreign_resource).is_err());

        // AuthZ layer also blocks it.
        let claims = TokenClaims {
            tenant_id: "acme".into(),
            role: Role::Admin,
            scoped_permissions: None,
        };
        assert!(evaluate_authz(&claims, &foreign_resource, Permission::Read).is_err());
    }
}
