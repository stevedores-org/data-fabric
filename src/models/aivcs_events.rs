//! Canonical AIVCS event_type constants. Use these instead of string literals.
//!
//! These constants declare the AIVCS event taxonomy from issue #148. They are
//! informational: callers should reference them instead of inline string
//! literals so we can audit which event types AIVCS expects. There is no
//! runtime gate (whitelist) against this list yet — that is a separate
//! decision tracked under issue #148.

// AIVCS lifecycle events
pub const AIVCS_REPOSITORY_SYNCED: &str = "aivcs.repository.synced";
pub const AIVCS_BRANCH_CREATED: &str = "aivcs.branch.created";
pub const AIVCS_COMMIT_INGESTED: &str = "aivcs.commit.ingested";
pub const AIVCS_CHANGE_SET_PROPOSED: &str = "aivcs.change_set.proposed";
pub const AIVCS_DIFF_ARTIFACT_STORED: &str = "aivcs.diff.artifact_stored";
pub const AIVCS_REVIEW_OPENED: &str = "aivcs.review.opened";
pub const AIVCS_REVIEW_THREAD_CREATED: &str = "aivcs.review_thread.created";

// Agent events
pub const AGENT_INTENT_RECORDED: &str = "agent.intent.recorded";
pub const AGENT_PLAN_CREATED: &str = "agent.plan.created";
pub const AGENT_TOOL_REQUESTED: &str = "agent.tool.requested";
pub const AGENT_TOOL_COMPLETED: &str = "agent.tool.completed";
pub const AGENT_CONFIDENCE_UPDATED: &str = "agent.confidence.updated";
pub const AGENT_RISK_UPDATED: &str = "agent.risk.updated";

// CI events
pub const CI_CHECK_STARTED: &str = "ci.check.started";
pub const CI_CHECK_COMPLETED: &str = "ci.check.completed";
pub const CI_CHECK_FAILED: &str = "ci.check.failed";

// Policy events
pub const POLICY_CHECK_REQUESTED: &str = "policy.check.requested";
pub const POLICY_DECISION_RECORDED: &str = "policy.decision.recorded";
pub const POLICY_ESCALATION_CREATED: &str = "policy.escalation.created";

// Human events
pub const HUMAN_GUIDANCE_ADDED: &str = "human.guidance.added";
pub const HUMAN_CONSTRAINT_ADDED: &str = "human.constraint.added";
pub const HUMAN_REVIEW_COMMENT_ADDED: &str = "human.review.comment_added";
pub const HUMAN_APPROVED: &str = "human.approved";
pub const HUMAN_REQUESTED_CHANGES: &str = "human.requested_changes";
pub const HUMAN_PAUSED_AGENT: &str = "human.paused_agent";
pub const HUMAN_RESUMED_AGENT: &str = "human.resumed_agent";
pub const HUMAN_MERGE_REQUESTED: &str = "human.merge_requested";

// Merge / release events
pub const AIVCS_MERGE_GUARDRAILS_STARTED: &str = "aivcs.merge.guardrails_started";
pub const AIVCS_MERGE_BLOCKED: &str = "aivcs.merge.blocked";
pub const AIVCS_MERGE_COMPLETED: &str = "aivcs.merge.completed";
pub const AIVCS_RELEASE_CREATED: &str = "aivcs.release.created";

/// All canonical AIVCS event_type constants, declared in stable order.
///
/// Iterate over this slice for documentation, snapshot tests, or to feed an
/// allowlist later. Do NOT take a runtime dependency on the ordering for
/// correctness — only for human-readable output / grep-ability.
pub const ALL: &[&str] = &[
    // AIVCS lifecycle
    AIVCS_REPOSITORY_SYNCED,
    AIVCS_BRANCH_CREATED,
    AIVCS_COMMIT_INGESTED,
    AIVCS_CHANGE_SET_PROPOSED,
    AIVCS_DIFF_ARTIFACT_STORED,
    AIVCS_REVIEW_OPENED,
    AIVCS_REVIEW_THREAD_CREATED,
    // Agent
    AGENT_INTENT_RECORDED,
    AGENT_PLAN_CREATED,
    AGENT_TOOL_REQUESTED,
    AGENT_TOOL_COMPLETED,
    AGENT_CONFIDENCE_UPDATED,
    AGENT_RISK_UPDATED,
    // CI
    CI_CHECK_STARTED,
    CI_CHECK_COMPLETED,
    CI_CHECK_FAILED,
    // Policy
    POLICY_CHECK_REQUESTED,
    POLICY_DECISION_RECORDED,
    POLICY_ESCALATION_CREATED,
    // Human
    HUMAN_GUIDANCE_ADDED,
    HUMAN_CONSTRAINT_ADDED,
    HUMAN_REVIEW_COMMENT_ADDED,
    HUMAN_APPROVED,
    HUMAN_REQUESTED_CHANGES,
    HUMAN_PAUSED_AGENT,
    HUMAN_RESUMED_AGENT,
    HUMAN_MERGE_REQUESTED,
    // Merge / release
    AIVCS_MERGE_GUARDRAILS_STARTED,
    AIVCS_MERGE_BLOCKED,
    AIVCS_MERGE_COMPLETED,
    AIVCS_RELEASE_CREATED,
];

/// AIVCS-owned event_type namespace prefixes.
///
/// An event_type belongs to the AIVCS taxonomy if it starts with any of these
/// prefixes. This is informational only — it is NOT used as a runtime gate.
/// Callers may use it for auditing, dashboards, or routing.
const AIVCS_PREFIXES: &[&str] = &[
    "aivcs.",
    "agent.intent.",
    "agent.plan.",
    "agent.tool.",
    "agent.confidence.",
    "agent.risk.",
    "ci.check.",
    "policy.",
    "human.",
];

/// Returns true if `event_type` belongs to the AIVCS taxonomy.
///
/// Informational only — do not use as an authorization gate or as a
/// validation step on inbound events. The set of AIVCS-owned namespaces is
/// fixed by issue #148.
pub fn is_aivcs_event(event_type: &str) -> bool {
    AIVCS_PREFIXES.iter().any(|p| event_type.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_constant_is_unique() {
        let set: HashSet<&&str> = ALL.iter().collect();
        assert_eq!(
            set.len(),
            ALL.len(),
            "duplicate event_type constants in ALL: {:?}",
            ALL
        );
    }

    #[test]
    fn every_constant_follows_dot_lowercase_pattern() {
        for c in ALL {
            assert!(!c.is_empty(), "constant must not be empty");
            assert!(
                c.contains('.'),
                "constant {:?} must contain at least one '.'",
                c
            );
            assert!(
                !c.chars().any(|ch| ch.is_whitespace()),
                "constant {:?} must not contain whitespace",
                c
            );
            assert!(
                !c.chars().any(|ch| ch.is_ascii_uppercase()),
                "constant {:?} must not contain uppercase ASCII",
                c
            );
            // Reject any non-ASCII to keep grep simple.
            assert!(c.is_ascii(), "constant {:?} must be ASCII", c);
            // No leading or trailing dot, and no empty segments.
            assert!(
                !c.starts_with('.') && !c.ends_with('.'),
                "constant {:?} must not start or end with '.'",
                c
            );
            for seg in c.split('.') {
                assert!(
                    !seg.is_empty(),
                    "constant {:?} must not contain empty dot-separated segments",
                    c
                );
            }
        }
    }

    #[test]
    fn is_aivcs_event_true_for_every_constant() {
        for c in ALL {
            assert!(
                is_aivcs_event(c),
                "is_aivcs_event returned false for canonical constant {:?}",
                c
            );
        }
    }

    #[test]
    fn is_aivcs_event_false_for_non_aivcs_event_types() {
        // These are known non-AIVCS event types from the existing WS2/MCP
        // taxonomy. They must NOT match the AIVCS prefixes.
        let non_aivcs = [
            "run.created",
            "run.completed",
            "task.created",
            "plan.created",
            "tool_call.completed",
            "artifact.stored",
            "release.created",
            "checkpoint.taken",
            "graph.event",
            "",
            "aivcs",             // missing trailing dot — should not match prefix "aivcs."
            "agent",             // bare "agent" is not an AIVCS-owned namespace
            "agent.other.thing", // "agent.other." is not one of the AIVCS sub-namespaces
            "ci.other.thing",    // only "ci.check." is AIVCS-owned
        ];
        for ev in non_aivcs {
            assert!(
                !is_aivcs_event(ev),
                "is_aivcs_event returned true for non-AIVCS event {:?}",
                ev
            );
        }
    }

    /// Snapshot: prints every constant in declaration order so a future
    /// contributor can `cargo test -- --nocapture event_taxonomy_snapshot`
    /// and grep the taxonomy. Acts as a checked inventory: if a constant is
    /// removed from ALL but kept in source, this test still runs against
    /// whatever ALL contains.
    #[test]
    fn event_taxonomy_snapshot() {
        let mut out = String::new();
        out.push_str("AIVCS event taxonomy (issue #148):\n");
        for c in ALL {
            out.push_str(c);
            out.push('\n');
        }
        // Print for `--nocapture` consumers.
        println!("{}", out);
        // Pin the count so accidental removals fail loudly. Update this
        // number deliberately when adding/removing constants.
        assert_eq!(ALL.len(), 31, "ALL count changed; update the snapshot");
    }
}
