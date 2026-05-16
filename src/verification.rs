use crate::models;

pub fn compute_replay_drift_percent(
    steps: &[models::ReplayStep],
    baseline_steps: &[models::ReplayStep],
) -> (usize, f64) {
    let compared = steps.len().max(baseline_steps.len());
    if compared == 0 {
        return (0, 0.0);
    }

    let mut drift_count = 0usize;
    for index in 0..compared {
        let left = steps.get(index);
        let right = baseline_steps.get(index);
        let mismatch = match (left, right) {
            (Some(a), Some(b)) => {
                a.event_type != b.event_type || a.node_id != b.node_id || a.actor != b.actor
            }
            _ => true,
        };
        if mismatch {
            drift_count += 1;
        }
    }

    let drift_ratio_percent =
        ((drift_count as f64 / compared as f64) * 100.0 * 100.0).round() / 100.0;
    (drift_count, drift_ratio_percent)
}

pub fn classify_failure_from_drift_ratio(drift_ratio_percent: f64) -> models::FailureClass {
    if drift_ratio_percent <= f64::EPSILON {
        models::FailureClass::Deterministic
    } else if drift_ratio_percent <= 10.0 {
        models::FailureClass::Transient
    } else if drift_ratio_percent <= 35.0 {
        models::FailureClass::Environmental
    } else {
        models::FailureClass::Logical
    }
}

pub fn evaluate_verification_gates(
    tests_passed: bool,
    policy_approved: bool,
    provenance_complete: bool,
) -> models::VerificationGateResult {
    let mut confidence_score = 0u8;
    let mut failed_gates = Vec::new();

    if tests_passed {
        confidence_score += 40;
    } else {
        failed_gates.push("tests_passed".to_string());
    }

    if policy_approved {
        confidence_score += 35;
    } else {
        failed_gates.push("policy_approved".to_string());
    }

    if provenance_complete {
        confidence_score += 25;
    } else {
        failed_gates.push("provenance_complete".to_string());
    }

    models::VerificationGateResult {
        tests_passed,
        policy_approved,
        provenance_complete,
        eligible_for_promotion: tests_passed && policy_approved && provenance_complete,
        confidence_score,
        failed_gates,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_failure_from_drift_ratio, compute_replay_drift_percent,
        evaluate_verification_gates,
    };
    use crate::models;

    #[test]
    fn compute_replay_drift_percent_zero_for_identical_steps() {
        let steps = vec![models::ReplayStep {
            sequence: 1,
            event_type: "node.start".into(),
            node_id: Some("n1".into()),
            actor: Some("agent".into()),
        }];
        let (drift_count, drift_ratio) = compute_replay_drift_percent(&steps, &steps);
        assert_eq!(drift_count, 0);
        assert!((drift_ratio - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn compute_replay_drift_percent_counts_mismatch_and_length_delta() {
        let replay = vec![
            models::ReplayStep {
                sequence: 1,
                event_type: "run.start".into(),
                node_id: None,
                actor: Some("ci".into()),
            },
            models::ReplayStep {
                sequence: 2,
                event_type: "node.start".into(),
                node_id: Some("n1".into()),
                actor: Some("agent".into()),
            },
        ];
        let baseline = vec![models::ReplayStep {
            sequence: 1,
            event_type: "run.start".into(),
            node_id: None,
            actor: Some("ci".into()),
        }];
        let (drift_count, drift_ratio) = compute_replay_drift_percent(&replay, &baseline);
        assert_eq!(drift_count, 1);
        assert!((drift_ratio - 50.0).abs() < 0.001);
    }

    #[test]
    fn classify_failure_from_drift_ratio_boundaries() {
        assert_eq!(
            classify_failure_from_drift_ratio(0.0),
            models::FailureClass::Deterministic
        );
        assert_eq!(
            classify_failure_from_drift_ratio(5.0),
            models::FailureClass::Transient
        );
        assert_eq!(
            classify_failure_from_drift_ratio(20.0),
            models::FailureClass::Environmental
        );
        assert_eq!(
            classify_failure_from_drift_ratio(90.0),
            models::FailureClass::Logical
        );
    }

    #[test]
    fn evaluate_verification_gates_all_pass() {
        let result = evaluate_verification_gates(true, true, true);
        assert!(result.eligible_for_promotion);
        assert_eq!(result.confidence_score, 100);
        assert!(result.failed_gates.is_empty());
    }

    #[test]
    fn evaluate_verification_gates_collects_failures() {
        let result = evaluate_verification_gates(false, true, false);
        assert!(!result.eligible_for_promotion);
        assert_eq!(result.confidence_score, 35);
        assert_eq!(
            result.failed_gates,
            vec!["tests_passed", "provenance_complete"]
        );
    }

    #[test]
    fn evaluate_verification_gates_all_fail() {
        let result = evaluate_verification_gates(false, false, false);
        assert!(!result.eligible_for_promotion);
        assert_eq!(result.confidence_score, 0);
        assert_eq!(result.failed_gates.len(), 3);
    }

    #[test]
    fn compute_replay_drift_percent_empty_steps() {
        let empty: Vec<models::ReplayStep> = vec![];
        let (drift_count, drift_ratio) = compute_replay_drift_percent(&empty, &empty);
        assert_eq!(drift_count, 0);
        assert!((drift_ratio - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn compute_replay_drift_percent_full_mismatch() {
        let replay = vec![
            models::ReplayStep { sequence: 1, event_type: "a".into(), node_id: None, actor: None },
            models::ReplayStep { sequence: 2, event_type: "b".into(), node_id: None, actor: None },
        ];
        let baseline = vec![
            models::ReplayStep { sequence: 1, event_type: "x".into(), node_id: None, actor: None },
            models::ReplayStep { sequence: 2, event_type: "y".into(), node_id: None, actor: None },
        ];
        let (drift_count, drift_ratio) = compute_replay_drift_percent(&replay, &baseline);
        assert_eq!(drift_count, 2);
        assert!((drift_ratio - 100.0).abs() < 0.001);
    }

    #[test]
    fn classify_failure_boundary_at_ten_percent() {
        // Exactly 10% should be Transient (<=10)
        assert_eq!(
            classify_failure_from_drift_ratio(10.0),
            models::FailureClass::Transient
        );
        // Just above should be Environmental
        assert_eq!(
            classify_failure_from_drift_ratio(10.01),
            models::FailureClass::Environmental
        );
    }

    #[test]
    fn classify_failure_boundary_at_thirtyfive_percent() {
        assert_eq!(
            classify_failure_from_drift_ratio(35.0),
            models::FailureClass::Environmental
        );
        assert_eq!(
            classify_failure_from_drift_ratio(35.01),
            models::FailureClass::Logical
        );
    }
}
