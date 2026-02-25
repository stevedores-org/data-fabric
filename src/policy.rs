use crate::db;
use crate::models;
use serde::{Deserialize, Serialize};
use worker::*;

const POLICY_KV_BINDING: &str = "POLICY_KV";
const ACTIVE_POLICY_VERSION_KEY: &str = "policy:active_version";
const POLICY_RULE_KEY_PREFIX: &str = "policy:rules:";
const POLICY_R2_KEY_PREFIX: &str = "policies/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleEffect {
    Allow,
    Deny,
    Escalate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub effect: RuleEffect,
    #[serde(default = "wildcard_all")]
    pub action: String,
    #[serde(default = "wildcard_all")]
    pub resource: String,
    #[serde(default = "wildcard_all")]
    pub actor: String,
    pub min_risk: Option<RiskLevel>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitRule {
    pub action_class: String,
    pub window_seconds: i64,
    pub max_requests: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBundle {
    pub version: String,
    pub rules: Vec<PolicyRule>,
    #[serde(default)]
    pub rate_limits: Vec<RateLimitRule>,
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub decision_id: String,
    pub decision: String,
    pub reason: String,
    pub risk_level: RiskLevel,
    pub policy_version: String,
    pub matched_rule: Option<String>,
    pub escalation_id: Option<String>,
    pub rate_limited: bool,
}

pub async fn evaluate_policy(
    env: &Env,
    d1: &D1Database,
    tenant_id: &str,
    req: &models::PolicyCheckRequest,
) -> Result<Decision> {
    let bundle = load_policy_bundle(env)
        .await
        .unwrap_or_else(|_| default_bundle());
    let risk = classify_risk(&req.action, req.resource.as_deref(), req.context.as_ref());
    let mut matched_rule: Option<String> = None;
    let mut rate_limited = false;

    let action_class = classify_action_class(&req.action, risk);
    let effective_rate = bundle
        .rate_limits
        .iter()
        .find(|r| wildcard_match(&r.action_class, &action_class))
        .cloned()
        .unwrap_or_else(|| default_rate_limit_for(risk));

    let exceeded = db::check_and_increment_rate_limit(
        d1,
        &req.actor,
        &action_class,
        effective_rate.window_seconds,
        effective_rate.max_requests,
    )
    .await?;
    if exceeded {
        rate_limited = true;
    }

    let mut verdict = if exceeded {
        RuleEffect::Escalate
    } else {
        RuleEffect::Allow
    };
    let mut reason = if exceeded {
        "rate limit exceeded for actor/action class".to_string()
    } else {
        "auto-approved low-risk operation".to_string()
    };

    if !exceeded {
        if let Some(rule) = first_matching_rule(&bundle.rules, req, risk) {
            matched_rule = Some(rule.id.clone());
            verdict = rule.effect.clone();
            reason = rule.reason.clone();
        } else if risk >= RiskLevel::High {
            verdict = RuleEffect::Escalate;
            reason = "high-risk action requires explicit policy match".into();
        }
    }

    let decision_str = match verdict {
        RuleEffect::Allow => "allow",
        RuleEffect::Deny => "deny",
        RuleEffect::Escalate => "escalate",
    };
    let decision_id = random_hex_id()?;

    let escalation_id = if matches!(verdict, RuleEffect::Escalate) {
        let eid = random_hex_id()?;
        db::create_policy_escalation(
            d1,
            &eid,
            &decision_id,
            &req.action,
            &req.actor,
            req.resource.as_deref(),
            risk,
            req.context.as_ref(),
        )
        .await?;
        Some(eid)
    } else {
        None
    };

    db::record_policy_check_detailed(
        d1,
        tenant_id,
        &decision_id,
        req,
        decision_str,
        &reason,
        risk,
        &bundle.version,
        matched_rule.as_deref(),
        escalation_id.as_deref(),
        rate_limited,
    )
    .await?;

    Ok(Decision {
        decision_id,
        decision: decision_str.into(),
        reason,
        risk_level: risk,
        policy_version: bundle.version,
        matched_rule,
        escalation_id,
        rate_limited,
    })
}

pub async fn put_policy_definition(
    env: &Env,
    bucket: &Bucket,
    version: &str,
    req: &models::PutPolicyDefinitionRequest,
) -> Result<models::PolicyDefinitionResponse> {
    // Validate schema by deserializing.
    let mut bundle: PolicyBundle = serde_json::from_value(req.bundle.clone())
        .map_err(|e| Error::RustError(format!("invalid policy bundle: {e}")))?;
    if bundle.version.is_empty() {
        bundle.version = version.to_string();
    }
    if bundle.version != version {
        return Err(Error::RustError(
            "bundle.version must match path version".to_string(),
        ));
    }

    let payload = serde_json::to_vec(&bundle)
        .map_err(|e| Error::RustError(format!("serialize policy bundle: {e}")))?;
    let r2_key = format!("{POLICY_R2_KEY_PREFIX}{version}.json");
    bucket.put(&r2_key, payload).execute().await?;

    // Best effort KV write; if binding absent, still keep R2 as source of truth.
    let activated = if let Ok(kv) = env.kv(POLICY_KV_BINDING) {
        let kv_key = format!("{POLICY_RULE_KEY_PREFIX}{version}");
        let text = serde_json::to_string(&bundle).unwrap_or_else(|_| "{}".to_string());
        kv.put(&kv_key, text)?.execute().await?;
        if req.activate {
            kv.put(ACTIVE_POLICY_VERSION_KEY, version)?
                .execute()
                .await?;
            true
        } else {
            false
        }
    } else {
        if req.activate {
            worker::console_log!(
                "WARN: policy activation requested but POLICY_KV binding is absent; \
                 stored to R2 only — use POST /v1/policies/activate/:version after provisioning KV"
            );
        }
        false
    };

    Ok(models::PolicyDefinitionResponse {
        version: version.to_string(),
        stored: true,
        activated,
    })
}

pub async fn activate_policy_version(
    env: &Env,
    version: &str,
) -> Result<models::PolicyActivationResponse> {
    let kv = env.kv(POLICY_KV_BINDING)?;
    kv.put(ACTIVE_POLICY_VERSION_KEY, version)?
        .execute()
        .await?;
    Ok(models::PolicyActivationResponse {
        version: version.to_string(),
        active: true,
    })
}

pub async fn active_policy_version(env: &Env) -> Result<models::ActivePolicyResponse> {
    match env.kv(POLICY_KV_BINDING) {
        Ok(kv) => {
            let version = kv
                .get(ACTIVE_POLICY_VERSION_KEY)
                .text()
                .await
                .ok()
                .flatten();
            Ok(models::ActivePolicyResponse {
                version,
                source: "kv".into(),
            })
        }
        Err(_) => Ok(models::ActivePolicyResponse {
            version: Some(default_bundle().version),
            source: "builtin".into(),
        }),
    }
}

pub fn classify_risk(
    action: &str,
    resource: Option<&str>,
    context: Option<&serde_json::Value>,
) -> RiskLevel {
    let a = action.to_ascii_lowercase();
    let r = resource.unwrap_or_default().to_ascii_lowercase();
    let c = context
        .map(|v| v.to_string().to_ascii_lowercase())
        .unwrap_or_default();
    let hay = format!("{a} {r} {c}");

    if has_any(
        &hay,
        &[
            "wipe",
            "destroy",
            "terminate",
            "root-key",
            "irreversible",
            "hard-delete",
        ],
    ) {
        return RiskLevel::Critical;
    }
    if has_any(
        &hay,
        &[
            "deploy",
            "delete",
            "drop",
            "credential",
            "secret",
            "production",
            "prod",
            "revoke",
            "merge-main",
            "push-main",
        ],
    ) {
        return RiskLevel::High;
    }
    if has_any(
        &hay,
        &[
            "create", "update", "write", "put", "patch", "commit", "index",
        ],
    ) {
        return RiskLevel::Medium;
    }
    if has_any(
        &hay,
        &[
            "read", "get", "list", "query", "search", "status", "health", "trace",
        ],
    ) {
        return RiskLevel::Low;
    }
    RiskLevel::Medium
}

fn classify_action_class(action: &str, risk: RiskLevel) -> String {
    let a = action.to_ascii_lowercase();
    if a.contains("deploy") {
        "deploy".into()
    } else if a.contains("delete") || a.contains("drop") {
        "delete".into()
    } else {
        match risk {
            RiskLevel::Low => "read".into(),
            RiskLevel::Medium => "write".into(),
            RiskLevel::High => "high_risk".into(),
            RiskLevel::Critical => "critical".into(),
        }
    }
}

fn default_rate_limit_for(risk: RiskLevel) -> RateLimitRule {
    match risk {
        RiskLevel::Low => RateLimitRule {
            action_class: "read".into(),
            window_seconds: 60,
            max_requests: 240,
        },
        RiskLevel::Medium => RateLimitRule {
            action_class: "write".into(),
            window_seconds: 60,
            max_requests: 120,
        },
        RiskLevel::High => RateLimitRule {
            action_class: "high_risk".into(),
            window_seconds: 60,
            max_requests: 30,
        },
        RiskLevel::Critical => RateLimitRule {
            action_class: "critical".into(),
            window_seconds: 60,
            max_requests: 10,
        },
    }
}

fn first_matching_rule<'a>(
    rules: &'a [PolicyRule],
    req: &models::PolicyCheckRequest,
    risk: RiskLevel,
) -> Option<&'a PolicyRule> {
    let action = req.action.to_ascii_lowercase();
    let resource = req
        .resource
        .clone()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let actor = req.actor.to_ascii_lowercase();

    rules.iter().find(|rule| {
        if let Some(min) = rule.min_risk {
            if risk < min {
                return false;
            }
        }
        wildcard_match(&rule.action, &action)
            && wildcard_match(&rule.resource, &resource)
            && wildcard_match(&rule.actor, &actor)
    })
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let p = pattern.to_ascii_lowercase();
    let v = value.to_ascii_lowercase();
    if !p.contains('*') {
        return p == v;
    }
    let parts: Vec<&str> = p.split('*').collect();
    let mut pos = 0usize;
    for part in parts.iter().filter(|s| !s.is_empty()) {
        if let Some(found) = v[pos..].find(part) {
            pos += found + part.len();
        } else {
            return false;
        }
    }
    true
}

async fn load_policy_bundle(env: &Env) -> Result<PolicyBundle> {
    // Try KV first (hot path).
    if let Ok(kv) = env.kv(POLICY_KV_BINDING) {
        let version = kv
            .get(ACTIVE_POLICY_VERSION_KEY)
            .text()
            .await?
            .unwrap_or_else(|| "builtin-2026-02-22".to_string());
        let key = format!("{POLICY_RULE_KEY_PREFIX}{version}");
        if let Some(text) = kv.get(&key).text().await? {
            let bundle: PolicyBundle = serde_json::from_str(&text)
                .map_err(|e| Error::RustError(format!("invalid policy json: {e}")))?;
            return Ok(bundle);
        }
    }
    // KV absent or active version key missing — fall back to builtin defaults.
    // R2 holds durable policy blobs but is not used for evaluation without a
    // version pointer (which lives in KV). KV = hot-path read; R2 = archive.
    Ok(default_bundle())
}

fn default_bundle() -> PolicyBundle {
    PolicyBundle {
        version: "builtin-2026-02-22".into(),
        rules: vec![
            PolicyRule {
                id: "deny-credential-exfiltration".into(),
                effect: RuleEffect::Deny,
                action: "*credential*".into(),
                resource: "*".into(),
                actor: "*".into(),
                min_risk: Some(RiskLevel::High),
                reason: "credential operations require dedicated secure channel".into(),
            },
            PolicyRule {
                id: "escalate-prod-deploy".into(),
                effect: RuleEffect::Escalate,
                action: "*deploy*".into(),
                resource: "*prod*".into(),
                actor: "*".into(),
                min_risk: Some(RiskLevel::High),
                reason: "production deploy requires human-in-the-loop approval".into(),
            },
            PolicyRule {
                id: "allow-read".into(),
                effect: RuleEffect::Allow,
                action: "*read*".into(),
                resource: "*".into(),
                actor: "*".into(),
                min_risk: Some(RiskLevel::Low),
                reason: "read-only actions are auto-approved".into(),
            },
        ],
        rate_limits: vec![
            RateLimitRule {
                action_class: "read".into(),
                window_seconds: 60,
                max_requests: 240,
            },
            RateLimitRule {
                action_class: "write".into(),
                window_seconds: 60,
                max_requests: 120,
            },
            RateLimitRule {
                action_class: "deploy".into(),
                window_seconds: 60,
                max_requests: 30,
            },
            RateLimitRule {
                action_class: "delete".into(),
                window_seconds: 60,
                max_requests: 20,
            },
        ],
    }
}

fn has_any(s: &str, terms: &[&str]) -> bool {
    terms.iter().any(|t| s.contains(t))
}

fn wildcard_all() -> String {
    "*".into()
}

fn random_hex_id() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)
        .map_err(|err| Error::RustError(format!("failed to generate id: {err}")))?;
    Ok(hex::encode(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify_risk ──────────────────────────────────────────

    #[test]
    fn risk_classification_matches_expected() {
        assert_eq!(
            classify_risk("read_status", Some("repo"), None),
            RiskLevel::Low
        );
        assert_eq!(
            classify_risk("update_config", Some("repo"), None),
            RiskLevel::Medium
        );
        assert_eq!(classify_risk("deploy", Some("prod"), None), RiskLevel::High);
        assert_eq!(
            classify_risk("hard-delete", Some("prod"), None),
            RiskLevel::Critical
        );
    }

    #[test]
    fn risk_critical_keywords() {
        for keyword in ["wipe", "destroy", "terminate", "root-key", "irreversible"] {
            assert_eq!(
                classify_risk(keyword, None, None),
                RiskLevel::Critical,
                "expected Critical for action '{keyword}'"
            );
        }
    }

    #[test]
    fn risk_high_keywords() {
        for keyword in [
            "deploy", "delete", "drop", "credential", "secret", "production", "prod", "revoke",
            "merge-main", "push-main",
        ] {
            assert_eq!(
                classify_risk(keyword, None, None),
                RiskLevel::High,
                "expected High for action '{keyword}'"
            );
        }
    }

    #[test]
    fn risk_medium_keywords() {
        for keyword in ["create", "update", "write", "put", "patch", "commit", "index"] {
            assert_eq!(
                classify_risk(keyword, None, None),
                RiskLevel::Medium,
                "expected Medium for action '{keyword}'"
            );
        }
    }

    #[test]
    fn risk_low_keywords() {
        for keyword in [
            "read", "get", "list", "query", "search", "status", "health", "trace",
        ] {
            assert_eq!(
                classify_risk(keyword, None, None),
                RiskLevel::Low,
                "expected Low for action '{keyword}'"
            );
        }
    }

    #[test]
    fn risk_unknown_action_defaults_to_medium() {
        assert_eq!(
            classify_risk("frobnicate", None, None),
            RiskLevel::Medium
        );
    }

    #[test]
    fn risk_is_case_insensitive() {
        assert_eq!(classify_risk("DEPLOY", None, None), RiskLevel::High);
        assert_eq!(classify_risk("Read", None, None), RiskLevel::Low);
        assert_eq!(classify_risk("HARD-DELETE", None, None), RiskLevel::Critical);
    }

    #[test]
    fn risk_picks_up_keywords_from_resource() {
        assert_eq!(
            classify_risk("unknown-action", Some("production-db"), None),
            RiskLevel::High,
        );
    }

    #[test]
    fn risk_picks_up_keywords_from_context() {
        let ctx = serde_json::json!({"env": "production"});
        assert_eq!(
            classify_risk("run-job", None, Some(&ctx)),
            RiskLevel::High,
        );
    }

    #[test]
    fn risk_critical_overrides_high() {
        // "hard-delete" is Critical, "prod" would be High — Critical wins.
        assert_eq!(
            classify_risk("hard-delete", Some("prod"), None),
            RiskLevel::Critical,
        );
    }

    // ── RiskLevel ordering ─────────────────────────────────────

    #[test]
    fn risk_level_ordering() {
        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
        assert!(RiskLevel::High < RiskLevel::Critical);
    }

    // ── classify_action_class ──────────────────────────────────

    #[test]
    fn action_class_deploy() {
        assert_eq!(classify_action_class("deploy-prod", RiskLevel::High), "deploy");
    }

    #[test]
    fn action_class_delete() {
        assert_eq!(classify_action_class("delete-user", RiskLevel::High), "delete");
        assert_eq!(classify_action_class("drop-table", RiskLevel::High), "delete");
    }

    #[test]
    fn action_class_falls_back_to_risk() {
        assert_eq!(classify_action_class("frobnicate", RiskLevel::Low), "read");
        assert_eq!(classify_action_class("frobnicate", RiskLevel::Medium), "write");
        assert_eq!(classify_action_class("frobnicate", RiskLevel::High), "high_risk");
        assert_eq!(classify_action_class("frobnicate", RiskLevel::Critical), "critical");
    }

    // ── default_rate_limit_for ─────────────────────────────────

    #[test]
    fn default_rate_limits_scale_with_risk() {
        let low = default_rate_limit_for(RiskLevel::Low);
        let med = default_rate_limit_for(RiskLevel::Medium);
        let high = default_rate_limit_for(RiskLevel::High);
        let crit = default_rate_limit_for(RiskLevel::Critical);

        assert!(low.max_requests > med.max_requests);
        assert!(med.max_requests > high.max_requests);
        assert!(high.max_requests > crit.max_requests);
    }

    #[test]
    fn default_rate_limit_action_classes() {
        assert_eq!(default_rate_limit_for(RiskLevel::Low).action_class, "read");
        assert_eq!(default_rate_limit_for(RiskLevel::Medium).action_class, "write");
        assert_eq!(default_rate_limit_for(RiskLevel::High).action_class, "high_risk");
        assert_eq!(default_rate_limit_for(RiskLevel::Critical).action_class, "critical");
    }

    // ── wildcard_match ─────────────────────────────────────────

    #[test]
    fn wildcard_matching_works() {
        assert!(wildcard_match("*deploy*", "safe_deploy_prod"));
        assert!(wildcard_match("agent-*", "agent-1"));
        assert!(!wildcard_match("agent-*", "service-1"));
    }

    #[test]
    fn wildcard_star_matches_everything() {
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("*", ""));
    }

    #[test]
    fn wildcard_exact_match_no_star() {
        assert!(wildcard_match("hello", "hello"));
        assert!(!wildcard_match("hello", "world"));
    }

    #[test]
    fn wildcard_case_insensitive() {
        assert!(wildcard_match("DEPLOY*", "deploy-prod"));
        assert!(wildcard_match("*PROD", "staging-prod"));
    }

    #[test]
    fn wildcard_multiple_stars() {
        assert!(wildcard_match("*deploy*prod*", "safe_deploy_to_prod_env"));
        assert!(!wildcard_match("*deploy*prod*", "safe_stage_to_dev_env"));
    }

    #[test]
    fn wildcard_empty_pattern_and_value() {
        assert!(wildcard_match("", ""));
        assert!(!wildcard_match("", "nonempty"));
    }

    // ── first_matching_rule ────────────────────────────────────

    fn make_request(action: &str, actor: &str, resource: Option<&str>) -> models::PolicyCheckRequest {
        models::PolicyCheckRequest {
            action: action.to_string(),
            actor: actor.to_string(),
            resource: resource.map(|s| s.to_string()),
            context: None,
            run_id: None,
        }
    }

    #[test]
    fn first_matching_rule_matches_action_wildcard() {
        let rules = vec![PolicyRule {
            id: "deny-all-deploys".into(),
            effect: RuleEffect::Deny,
            action: "*deploy*".into(),
            resource: "*".into(),
            actor: "*".into(),
            min_risk: None,
            reason: "no deploys".into(),
        }];
        let req = make_request("deploy-prod", "user-1", None);
        let matched = first_matching_rule(&rules, &req, RiskLevel::High);
        assert_eq!(matched.unwrap().id, "deny-all-deploys");
    }

    #[test]
    fn first_matching_rule_skips_below_min_risk() {
        let rules = vec![PolicyRule {
            id: "high-only".into(),
            effect: RuleEffect::Deny,
            action: "*".into(),
            resource: "*".into(),
            actor: "*".into(),
            min_risk: Some(RiskLevel::High),
            reason: "only high risk".into(),
        }];
        let req = make_request("read-file", "user-1", None);
        // Low risk should not match a rule with min_risk=High
        assert!(first_matching_rule(&rules, &req, RiskLevel::Low).is_none());
        // High risk should match
        assert!(first_matching_rule(&rules, &req, RiskLevel::High).is_some());
    }

    #[test]
    fn first_matching_rule_returns_first_match() {
        let rules = vec![
            PolicyRule {
                id: "first".into(),
                effect: RuleEffect::Allow,
                action: "*".into(),
                resource: "*".into(),
                actor: "*".into(),
                min_risk: None,
                reason: "first rule".into(),
            },
            PolicyRule {
                id: "second".into(),
                effect: RuleEffect::Deny,
                action: "*".into(),
                resource: "*".into(),
                actor: "*".into(),
                min_risk: None,
                reason: "second rule".into(),
            },
        ];
        let req = make_request("anything", "anyone", None);
        assert_eq!(
            first_matching_rule(&rules, &req, RiskLevel::Medium).unwrap().id,
            "first"
        );
    }

    #[test]
    fn first_matching_rule_filters_by_actor() {
        let rules = vec![PolicyRule {
            id: "admin-only".into(),
            effect: RuleEffect::Allow,
            action: "*".into(),
            resource: "*".into(),
            actor: "admin-*".into(),
            min_risk: None,
            reason: "admin only".into(),
        }];
        let req_user = make_request("read", "user-1", None);
        assert!(first_matching_rule(&rules, &req_user, RiskLevel::Low).is_none());

        let req_admin = make_request("read", "admin-bob", None);
        assert!(first_matching_rule(&rules, &req_admin, RiskLevel::Low).is_some());
    }

    #[test]
    fn first_matching_rule_no_rules_returns_none() {
        let req = make_request("read", "user-1", None);
        assert!(first_matching_rule(&[], &req, RiskLevel::Low).is_none());
    }

    // ── default_bundle ─────────────────────────────────────────

    #[test]
    fn default_bundle_has_expected_structure() {
        let bundle = default_bundle();
        assert!(!bundle.version.is_empty());
        assert!(!bundle.rules.is_empty());
        assert!(!bundle.rate_limits.is_empty());

        // Verify known rule IDs exist
        let rule_ids: Vec<&str> = bundle.rules.iter().map(|r| r.id.as_str()).collect();
        assert!(rule_ids.contains(&"deny-credential-exfiltration"));
        assert!(rule_ids.contains(&"escalate-prod-deploy"));
        assert!(rule_ids.contains(&"allow-read"));
    }

    // ── serde round-trips ──────────────────────────────────────

    #[test]
    fn risk_level_serde_roundtrip() {
        for level in [RiskLevel::Low, RiskLevel::Medium, RiskLevel::High, RiskLevel::Critical] {
            let json = serde_json::to_string(&level).unwrap();
            let back: RiskLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(back, level);
        }
    }

    #[test]
    fn risk_level_serde_snake_case() {
        assert_eq!(serde_json::to_string(&RiskLevel::Low).unwrap(), "\"low\"");
        assert_eq!(serde_json::to_string(&RiskLevel::Critical).unwrap(), "\"critical\"");
    }

    #[test]
    fn policy_bundle_serde_roundtrip() {
        let bundle = default_bundle();
        let json = serde_json::to_string(&bundle).unwrap();
        let back: PolicyBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, bundle.version);
        assert_eq!(back.rules.len(), bundle.rules.len());
        assert_eq!(back.rate_limits.len(), bundle.rate_limits.len());
    }

    #[test]
    fn policy_rule_default_fields_are_wildcard() {
        let json = r#"{"id":"test","effect":"allow","reason":"test rule"}"#;
        let rule: PolicyRule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.action, "*");
        assert_eq!(rule.resource, "*");
        assert_eq!(rule.actor, "*");
    }
}
