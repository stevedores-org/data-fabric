//! WS10 baseline pilot metrics.
//!
//! Implements the `GET /v1/metrics/pilot` endpoint contract (issue #105):
//! six KPIs computed from D1 over a rolling time window, with explicit
//! `null` + reason when a KPI cannot be computed. No zero sentinels.
//!
//! The aggregation logic is split into a pure function (`aggregate_kpis`)
//! that takes a `SampleCounts` + `RawAggregates` fixture and returns
//! a `Kpis` struct + `Reasons` map, so it can be tested without D1.

use serde::Serialize;
use wasm_bindgen::JsValue;
use worker::*;

/// Counts of source rows considered for KPI computation in the window.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize)]
pub struct SampleCounts {
    pub tasks: i64,
    pub events: i64,
    pub decisions: i64,
}

/// Aggregated KPI values. Each field is `Option<f64>` so we can return
/// `null` with an explicit reason instead of a misleading zero sentinel.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize)]
pub struct Kpis {
    pub task_completion_rate: Option<f64>,
    pub mttr_p50_seconds: Option<f64>,
    pub mttr_p95_seconds: Option<f64>,
    pub context_reuse_rate: Option<f64>,
    pub human_intervention_rate: Option<f64>,
    pub event_throughput_per_sec: Option<f64>,
}

/// Map of KPI key -> human-readable reason when the KPI is `null`.
#[allow(dead_code)]
pub type Reasons = std::collections::BTreeMap<String, String>;

#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize)]
pub struct Meta {
    pub generated_at: String,
    pub tenant_id: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct PilotMetricsResponse {
    pub window: String,
    pub window_seconds: i64,
    pub sample_counts: SampleCounts,
    pub kpis: Kpis,
    pub reasons: Reasons,
    pub meta: Meta,
}

/// Raw aggregates pulled from D1 (or a test fixture). All optional so the
/// caller can signal "could not compute" per KPI source.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct RawAggregates {
    /// Total mcp_tasks rows in window.
    pub task_total: i64,
    /// mcp_tasks rows with status='completed' in window.
    pub task_completed: i64,
    /// p50 of failure -> next-completed delta (seconds), for the same run_id.
    pub mttr_p50_seconds: Option<f64>,
    /// p95 of the same.
    pub mttr_p95_seconds: Option<f64>,
    /// Number of failure->completed pairs that contributed to MTTR samples.
    pub mttr_sample_count: i64,
    /// Total policy_decisions rows in window.
    pub policy_decision_total: i64,
    /// policy_decisions rows with decision='escalate'.
    pub policy_decision_escalate: i64,
    /// Total events_bronze rows in window.
    pub event_total: i64,
}

/// Window parser: accepts `Nd`, `Nh`, `Nm`. Returns (canonical_label, seconds).
///
/// Examples:
///   "1d"  -> ("1d",  86_400)
///   "2h"  -> ("2h",   7_200)
///   "30m" -> ("30m",  1_800)
///
/// Bounded to [60s, 30d] to keep D1 queries cheap.
pub fn parse_window(input: &str) -> std::result::Result<(String, i64), String> {
    const MIN_SECONDS: i64 = 60;
    const MAX_SECONDS: i64 = 30 * 86_400;

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("window is empty".into());
    }
    if trimmed.len() < 2 {
        return Err(format!("window '{trimmed}' must be like '1d', '2h', '30m'"));
    }
    let (num_part, unit) = trimmed.split_at(trimmed.len() - 1);
    let n: i64 = num_part
        .parse()
        .map_err(|_| format!("window '{trimmed}' has non-numeric prefix"))?;
    if n <= 0 {
        return Err(format!("window '{trimmed}' must be positive"));
    }
    let seconds = match unit {
        "s" => n,
        "m" => n.checked_mul(60).ok_or("window overflows")?,
        "h" => n.checked_mul(3_600).ok_or("window overflows")?,
        "d" => n.checked_mul(86_400).ok_or("window overflows")?,
        other => return Err(format!("window unit '{other}' not in [s,m,h,d]")),
    };
    if seconds < MIN_SECONDS {
        return Err(format!(
            "window '{trimmed}' below minimum of {MIN_SECONDS}s"
        ));
    }
    if seconds > MAX_SECONDS {
        return Err(format!(
            "window '{trimmed}' above maximum of {MAX_SECONDS}s (30d)"
        ));
    }
    Ok((trimmed.to_string(), seconds))
}

/// Pure aggregation: takes raw counts/aggregates plus the window length and
/// produces the (`Kpis`, `Reasons`) pair.
///
/// Returns `null` (and adds a reason entry) for any KPI whose denominator is
/// zero or whose underlying signal is missing. No zero sentinels.
pub fn aggregate_kpis(
    raw: &RawAggregates,
    window_seconds: i64,
    context_reuse_signal_available: bool,
) -> (Kpis, Reasons) {
    let mut kpis = Kpis::default();
    let mut reasons: Reasons = std::collections::BTreeMap::new();

    // task_completion_rate
    if raw.task_total > 0 {
        kpis.task_completion_rate = Some(raw.task_completed as f64 / raw.task_total as f64);
    } else {
        reasons.insert(
            "task_completion_rate".into(),
            "no mcp_tasks rows in window".into(),
        );
    }

    // MTTR p50 / p95
    if raw.mttr_sample_count > 0 {
        if let Some(p50) = raw.mttr_p50_seconds {
            kpis.mttr_p50_seconds = Some(p50);
        } else {
            reasons.insert(
                "mttr_p50_seconds".into(),
                "p50 computation returned no value despite samples present".into(),
            );
        }
        if let Some(p95) = raw.mttr_p95_seconds {
            kpis.mttr_p95_seconds = Some(p95);
        } else {
            reasons.insert(
                "mttr_p95_seconds".into(),
                "p95 computation returned no value despite samples present".into(),
            );
        }
    } else {
        reasons.insert(
            "mttr_p50_seconds".into(),
            "no failed->completed event pairs in window".into(),
        );
        reasons.insert(
            "mttr_p95_seconds".into(),
            "no failed->completed event pairs in window".into(),
        );
    }

    // context_reuse_rate — fuzzy KPI. Per spec, return null with explicit reason
    // when the run_summaries v1 schema lacks any cache-hit signal.
    if context_reuse_signal_available {
        // Reserved for a future schema that adds a checkpoint-hit column.
        // For now this branch is unreachable in production.
        kpis.context_reuse_rate = None;
        reasons.insert(
            "context_reuse_rate".into(),
            "context-reuse signal present but aggregation not implemented yet".into(),
        );
    } else {
        reasons.insert(
            "context_reuse_rate".into(),
            "no cache-hit signal in run_summaries v1".into(),
        );
    }

    // human_intervention_rate
    if raw.policy_decision_total > 0 {
        kpis.human_intervention_rate =
            Some(raw.policy_decision_escalate as f64 / raw.policy_decision_total as f64);
    } else {
        reasons.insert(
            "human_intervention_rate".into(),
            "no policy_decisions rows in window".into(),
        );
    }

    // event_throughput_per_sec
    if window_seconds > 0 && raw.event_total > 0 {
        kpis.event_throughput_per_sec = Some(raw.event_total as f64 / window_seconds as f64);
    } else if raw.event_total == 0 {
        reasons.insert(
            "event_throughput_per_sec".into(),
            "no events_bronze rows in window".into(),
        );
    } else {
        // window_seconds <= 0 should be impossible (parse_window enforces > 0)
        // but guard anyway to avoid div-by-zero in any future change.
        reasons.insert(
            "event_throughput_per_sec".into(),
            "window_seconds is non-positive".into(),
        );
    }

    (kpis, reasons)
}

// ── D1 helpers ─────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct ScalarI64 {
    v: i64,
}

#[derive(Debug, serde::Deserialize)]
struct ScalarF64Nullable {
    v: Option<f64>,
}

#[derive(Debug, serde::Deserialize)]
struct CompletionRow {
    total: i64,
    completed: i64,
}

#[derive(Debug, serde::Deserialize)]
struct PolicyRow {
    total: i64,
    escalate: i64,
}

/// SQLite expression returning the start-of-window ISO timestamp.
/// `datetime('now', '-N seconds')` returns `YYYY-MM-DD HH:MM:SS` format; we
/// compare against `created_at` columns that are stored as ISO-8601 strings.
/// Both formats sort lexicographically for the substring `YYYY-MM-DD HH:MM:SS`
/// up to the 'T' separator, so we use `strftime` to produce an explicit
/// ISO-8601 'T' form to be safe.
fn since_expr(window_seconds: i64) -> String {
    format!(
        "strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-{} seconds')",
        window_seconds
    )
}

/// Fetch raw aggregates for the pilot KPIs. Each KPI's query is independent
/// and tolerant of missing rows (returns zero/None).
pub async fn pilot_raw_aggregates(
    db: &D1Database,
    tenant_id: &str,
    window_seconds: i64,
    task_type: Option<&str>,
) -> Result<RawAggregates> {
    let since = since_expr(window_seconds);
    let mut raw = RawAggregates::default();

    // ── task_completion_rate: mcp_tasks ─────────────────────────
    let (task_sql, task_bindings): (String, Vec<JsValue>) = match task_type {
        Some(t) => (
            format!(
                "SELECT COUNT(*) AS total, \
                 SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS completed \
                 FROM mcp_tasks \
                 WHERE tenant_id = ?1 AND created_at >= {since} AND task_type = ?2"
            ),
            vec![JsValue::from_str(tenant_id), JsValue::from_str(t)],
        ),
        None => (
            format!(
                "SELECT COUNT(*) AS total, \
                 SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS completed \
                 FROM mcp_tasks \
                 WHERE tenant_id = ?1 AND created_at >= {since}"
            ),
            vec![JsValue::from_str(tenant_id)],
        ),
    };
    let task_row: Option<CompletionRow> = db
        .prepare(&task_sql)
        .bind(&task_bindings)?
        .first(None)
        .await?;
    if let Some(r) = task_row {
        raw.task_total = r.total.max(0);
        raw.task_completed = r.completed.max(0);
    }

    // ── MTTR: events_bronze, pair each failure with next completion ─
    //
    // SQLite (and therefore D1) supports window functions; we use LEAD()
    // partitioned by run_id and ordered by created_at to find the next
    // event after each failure. We treat `event_type LIKE '%fail%'` as a
    // failure marker and `event_type LIKE '%complet%'` as completion, since
    // integrations emit event_type names like `task.failed`, `run.completed`,
    // etc.
    //
    // The delta is computed in seconds via julianday() arithmetic, then we
    // ask SQLite to give us p50/p95 via NTILE in a wrapping CTE.
    let mttr_sql = format!(
        "WITH win AS ( \
            SELECT event_type, run_id, created_at, \
                   LEAD(event_type) OVER (PARTITION BY run_id ORDER BY created_at) AS next_type, \
                   LEAD(created_at) OVER (PARTITION BY run_id ORDER BY created_at) AS next_at \
            FROM events_bronze \
            WHERE tenant_id = ?1 AND created_at >= {since} AND run_id IS NOT NULL \
         ), \
         pairs AS ( \
            SELECT (julianday(next_at) - julianday(created_at)) * 86400.0 AS delta_s \
            FROM win \
            WHERE LOWER(event_type) LIKE '%fail%' \
              AND next_type IS NOT NULL \
              AND LOWER(next_type) LIKE '%complet%' \
              AND next_at > created_at \
         ) \
         SELECT COUNT(*) AS v FROM pairs"
    );
    let count_row: Option<ScalarI64> = db
        .prepare(&mttr_sql)
        .bind(&[JsValue::from_str(tenant_id)])?
        .first(None)
        .await?;
    let pair_count = count_row.map(|r| r.v.max(0)).unwrap_or(0);
    raw.mttr_sample_count = pair_count;

    if pair_count > 0 {
        // p50 and p95 via ORDER BY + LIMIT/OFFSET on the same CTE.
        // For p50 we take the value at index floor((n-1) * 0.5); for p95 floor((n-1) * 0.95).
        let p50_idx = ((pair_count - 1) as f64 * 0.50).floor() as i64;
        let p95_idx = ((pair_count - 1) as f64 * 0.95).floor() as i64;

        let pctile_sql = |offset: i64| -> String {
            format!(
                "WITH win AS ( \
                    SELECT event_type, run_id, created_at, \
                           LEAD(event_type) OVER (PARTITION BY run_id ORDER BY created_at) AS next_type, \
                           LEAD(created_at) OVER (PARTITION BY run_id ORDER BY created_at) AS next_at \
                    FROM events_bronze \
                    WHERE tenant_id = ?1 AND created_at >= {since} AND run_id IS NOT NULL \
                 ), \
                 pairs AS ( \
                    SELECT (julianday(next_at) - julianday(created_at)) * 86400.0 AS delta_s \
                    FROM win \
                    WHERE LOWER(event_type) LIKE '%fail%' \
                      AND next_type IS NOT NULL \
                      AND LOWER(next_type) LIKE '%complet%' \
                      AND next_at > created_at \
                 ) \
                 SELECT delta_s AS v FROM pairs ORDER BY delta_s ASC LIMIT 1 OFFSET {offset}"
            )
        };

        let p50_row: Option<ScalarF64Nullable> = db
            .prepare(pctile_sql(p50_idx))
            .bind(&[JsValue::from_str(tenant_id)])?
            .first(None)
            .await?;
        let p95_row: Option<ScalarF64Nullable> = db
            .prepare(pctile_sql(p95_idx))
            .bind(&[JsValue::from_str(tenant_id)])?
            .first(None)
            .await?;
        raw.mttr_p50_seconds = p50_row.and_then(|r| r.v);
        raw.mttr_p95_seconds = p95_row.and_then(|r| r.v);
    }

    // ── human_intervention_rate: policy_decisions ───────────────
    let policy_sql = format!(
        "SELECT COUNT(*) AS total, \
         SUM(CASE WHEN decision = 'escalate' THEN 1 ELSE 0 END) AS escalate \
         FROM policy_decisions \
         WHERE tenant_id = ?1 AND created_at >= {since}"
    );
    let policy_row: Option<PolicyRow> = db
        .prepare(&policy_sql)
        .bind(&[JsValue::from_str(tenant_id)])?
        .first(None)
        .await?;
    if let Some(r) = policy_row {
        raw.policy_decision_total = r.total.max(0);
        raw.policy_decision_escalate = r.escalate.max(0);
    }

    // ── event_throughput_per_sec: events_bronze ─────────────────
    let event_sql = format!(
        "SELECT COUNT(*) AS v FROM events_bronze \
         WHERE tenant_id = ?1 AND created_at >= {since}"
    );
    let event_row: Option<ScalarI64> = db
        .prepare(&event_sql)
        .bind(&[JsValue::from_str(tenant_id)])?
        .first(None)
        .await?;
    raw.event_total = event_row.map(|r| r.v.max(0)).unwrap_or(0);

    Ok(raw)
}

/// End-to-end: query D1 and assemble the response envelope.
pub async fn pilot_metrics(
    db: &D1Database,
    tenant_id: &str,
    window_label: &str,
    window_seconds: i64,
    task_type: Option<&str>,
    generated_at: String,
) -> Result<PilotMetricsResponse> {
    let raw = pilot_raw_aggregates(db, tenant_id, window_seconds, task_type).await?;

    // run_summaries v1 has no cache-hit column; signal explicitly absent.
    let context_reuse_signal_available = false;
    let (kpis, reasons) = aggregate_kpis(&raw, window_seconds, context_reuse_signal_available);

    let sample_counts = SampleCounts {
        tasks: raw.task_total,
        events: raw.event_total,
        decisions: raw.policy_decision_total,
    };

    Ok(PilotMetricsResponse {
        window: window_label.to_string(),
        window_seconds,
        sample_counts,
        kpis,
        reasons,
        meta: Meta {
            generated_at,
            tenant_id: tenant_id.to_string(),
        },
    })
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_window ────────────────────────────────────────────

    #[test]
    fn parse_window_accepts_canonical_units() {
        assert_eq!(parse_window("1d").unwrap(), ("1d".into(), 86_400));
        assert_eq!(parse_window("2h").unwrap(), ("2h".into(), 7_200));
        assert_eq!(parse_window("30m").unwrap(), ("30m".into(), 1_800));
        assert_eq!(parse_window("120s").unwrap(), ("120s".into(), 120));
    }

    #[test]
    fn parse_window_trims_input() {
        assert_eq!(parse_window("  1d  ").unwrap(), ("1d".into(), 86_400));
    }

    #[test]
    fn parse_window_rejects_invalid() {
        assert!(parse_window("").is_err());
        assert!(parse_window("abc").is_err());
        assert!(parse_window("1y").is_err());
        assert!(parse_window("0d").is_err());
        assert!(parse_window("-1d").is_err());
        // below the 60s floor
        assert!(parse_window("30s").is_err());
        // above the 30d ceiling
        assert!(parse_window("31d").is_err());
    }

    // ── aggregate_kpis ──────────────────────────────────────────

    fn fixture_full() -> RawAggregates {
        RawAggregates {
            task_total: 100,
            task_completed: 82,
            mttr_p50_seconds: Some(187.0),
            mttr_p95_seconds: Some(412.0),
            mttr_sample_count: 12,
            policy_decision_total: 50,
            policy_decision_escalate: 9,
            event_total: 8_900,
        }
    }

    #[test]
    fn aggregate_kpis_happy_path_reports_real_values() {
        let raw = fixture_full();
        let (kpis, reasons) = aggregate_kpis(&raw, 86_400, false);

        assert_eq!(kpis.task_completion_rate, Some(0.82));
        assert_eq!(kpis.mttr_p50_seconds, Some(187.0));
        assert_eq!(kpis.mttr_p95_seconds, Some(412.0));
        assert_eq!(kpis.human_intervention_rate, Some(0.18));
        // event_total / window_seconds
        let throughput = kpis.event_throughput_per_sec.unwrap();
        assert!((throughput - (8_900.0 / 86_400.0)).abs() < 1e-9);

        // context_reuse_rate is always null in v1
        assert!(kpis.context_reuse_rate.is_none());
        assert_eq!(
            reasons.get("context_reuse_rate").map(|s| s.as_str()),
            Some("no cache-hit signal in run_summaries v1"),
        );
        // No reasons for KPIs we computed
        assert!(!reasons.contains_key("task_completion_rate"));
        assert!(!reasons.contains_key("mttr_p50_seconds"));
        assert!(!reasons.contains_key("mttr_p95_seconds"));
        assert!(!reasons.contains_key("human_intervention_rate"));
        assert!(!reasons.contains_key("event_throughput_per_sec"));
    }

    #[test]
    fn aggregate_kpis_empty_window_returns_null_with_reasons() {
        let raw = RawAggregates::default();
        let (kpis, reasons) = aggregate_kpis(&raw, 86_400, false);

        // All KPIs should be None (no zero sentinels).
        assert!(kpis.task_completion_rate.is_none());
        assert!(kpis.mttr_p50_seconds.is_none());
        assert!(kpis.mttr_p95_seconds.is_none());
        assert!(kpis.context_reuse_rate.is_none());
        assert!(kpis.human_intervention_rate.is_none());
        assert!(kpis.event_throughput_per_sec.is_none());

        // Every KPI should have a reason entry.
        for key in [
            "task_completion_rate",
            "mttr_p50_seconds",
            "mttr_p95_seconds",
            "context_reuse_rate",
            "human_intervention_rate",
            "event_throughput_per_sec",
        ] {
            assert!(
                reasons.contains_key(key),
                "missing reason for {key}: {reasons:?}"
            );
        }
    }

    #[test]
    fn aggregate_kpis_partial_only_emits_reasons_for_missing() {
        // Tasks present but no events, no policy decisions, no MTTR samples.
        let raw = RawAggregates {
            task_total: 10,
            task_completed: 5,
            ..Default::default()
        };
        let (kpis, reasons) = aggregate_kpis(&raw, 3_600, false);
        assert_eq!(kpis.task_completion_rate, Some(0.5));
        assert!(kpis.event_throughput_per_sec.is_none());
        assert!(kpis.human_intervention_rate.is_none());
        assert!(kpis.mttr_p50_seconds.is_none());
        assert!(kpis.mttr_p95_seconds.is_none());

        assert!(!reasons.contains_key("task_completion_rate"));
        assert!(reasons.contains_key("event_throughput_per_sec"));
        assert!(reasons.contains_key("human_intervention_rate"));
        assert!(reasons.contains_key("mttr_p50_seconds"));
        assert!(reasons.contains_key("mttr_p95_seconds"));
    }
}
