//! WS10 pilot baseline KPIs (issue #105).
//!
//! Six KPIs surface through `GET /v1/metrics/pilot`. Five come from D1; the
//! sixth (API latency p50/p95/p99) is served separately via the Workers
//! Analytics Engine — see `scripts/pilot-latency.sh`.
//!
//! A KPI that has no underlying data in the window serializes as `null` with
//! an entry in the sibling `null_reasons` object. Zero is reserved for
//! "denominator was non-zero and numerator was actually zero" — never as a
//! stand-in for missing data, because the pilot-week go/no-go gates would
//! misread that.

use serde::Serialize;
use std::collections::BTreeMap;
use wasm_bindgen::JsValue;
use worker::*;

/// `event_type` values in `events_bronze` that we count as "the run hit a
/// failure state". Catalog isn't pinned anywhere central yet — this matches
/// what the WS3 provenance pipeline currently emits.
const FAILURE_EVENT_TYPES: &[&str] = &["run_failed", "task_failed", "error"];

/// `event_type` values that we count as "the run reached a healthy terminal
/// state" — used to close out MTTR intervals opened by a failure event.
const RECOVERY_EVENT_TYPES: &[&str] = &["run_completed", "task_completed", "recovered"];

#[derive(Debug, Serialize)]
pub struct PilotMetrics {
    pub window: String,
    pub window_seconds: i64,
    pub sample_counts: SampleCounts,
    pub kpis: Kpis,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub null_reasons: BTreeMap<&'static str, &'static str>,
    pub meta: Meta,
}

#[derive(Debug, Serialize)]
pub struct SampleCounts {
    pub tasks: i64,
    pub events: i64,
    pub decisions: i64,
}

#[derive(Debug, Default, Serialize)]
pub struct Kpis {
    pub task_completion_rate: Option<f64>,
    pub mttr_p50_seconds: Option<f64>,
    pub mttr_p95_seconds: Option<f64>,
    pub context_reuse_rate: Option<f64>,
    pub human_intervention_rate: Option<f64>,
    pub event_throughput_per_sec: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Meta {
    pub generated_at: String,
    pub tenant_id: String,
}

#[derive(Debug, serde::Deserialize)]
struct RateRow {
    numerator: i64,
    total: i64,
}

#[derive(Debug, serde::Deserialize)]
struct CountRow {
    total: i64,
}

#[derive(Debug, serde::Deserialize)]
struct EventRow {
    run_id: Option<String>,
    event_type: String,
    created_at: String,
}

/// Parse `?window=` to canonical `(echoed_string, seconds)`. Accepts `Nh`,
/// `Nd`, `Nw` for hours, days, weeks. `24h` and `1d` resolve to the same
/// `seconds` deliberately.
pub fn parse_window(raw: &str) -> Result<(String, i64)> {
    if raw.is_empty() {
        return Err(Error::RustError("window must not be empty".into()));
    }
    let (num_part, unit) = raw.split_at(raw.len() - 1);
    let n: i64 = num_part
        .parse()
        .map_err(|_| Error::RustError(format!("window: bad number in {raw:?}")))?;
    if n <= 0 {
        return Err(Error::RustError(format!(
            "window: must be positive, got {n}"
        )));
    }
    let secs_per_unit: i64 = match unit {
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        other => {
            return Err(Error::RustError(format!(
                "window: unknown unit {other:?} (expected h, d, or w)"
            )));
        }
    };
    let seconds = n
        .checked_mul(secs_per_unit)
        .ok_or_else(|| Error::RustError("window: overflow".into()))?;
    Ok((raw.to_string(), seconds))
}

/// Linear-interpolation percentile (NIST/Numpy default). Returns None on
/// empty input. Mutates nothing; expects `sorted` to be ascending.
pub fn percentile_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let p = p.clamp(0.0, 1.0);
    let rank = p * (sorted.len() as f64 - 1.0);
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        return Some(sorted[lo]);
    }
    let frac = rank - lo as f64;
    Some(sorted[lo] * (1.0 - frac) + sorted[hi] * frac)
}

/// Walk event rows (sorted by `(run_id, created_at ASC)`) and emit one
/// recovery-latency sample for each failure→recovery transition within a
/// single `run_id`. Per-run, the first failure starts the clock, the first
/// subsequent recovery stops it. After a recovery, the next failure can open
/// a new interval.
pub fn extract_mttr_deltas_seconds<I>(events: I) -> Vec<f64>
where
    I: IntoIterator<Item = (String, String, f64)>, // (run_id, event_type, epoch_seconds)
{
    let mut deltas = Vec::new();
    let mut current_run: Option<String> = None;
    let mut open_failure: Option<f64> = None;

    for (run_id, event_type, ts) in events {
        if Some(&run_id) != current_run.as_ref() {
            current_run = Some(run_id.clone());
            open_failure = None;
        }
        if FAILURE_EVENT_TYPES.contains(&event_type.as_str()) {
            if open_failure.is_none() {
                open_failure = Some(ts);
            }
        } else if RECOVERY_EVENT_TYPES.contains(&event_type.as_str()) {
            if let Some(start) = open_failure.take() {
                let delta = ts - start;
                if delta >= 0.0 {
                    deltas.push(delta);
                }
            }
        }
    }
    deltas
}

fn iso_to_epoch_seconds(iso: &str) -> Option<f64> {
    let date = js_sys::Date::new(&JsValue::from_str(iso));
    let ms = date.get_time();
    if ms.is_nan() {
        None
    } else {
        Some(ms / 1000.0)
    }
}

fn now_iso() -> String {
    js_sys::Date::new_0().to_iso_string().as_string().unwrap()
}

fn cutoff_iso(window_seconds: i64) -> String {
    let cutoff_ms = js_sys::Date::now() - (window_seconds as f64 * 1000.0);
    js_sys::Date::new(&JsValue::from_f64(cutoff_ms))
        .to_iso_string()
        .as_string()
        .unwrap()
}

/// Compute the pilot KPI block for one tenant over the given window.
pub async fn pilot(
    db: &D1Database,
    tenant_id: &str,
    window_raw: &str,
    window_seconds: i64,
    task_type: Option<&str>,
) -> Result<PilotMetrics> {
    let cutoff = cutoff_iso(window_seconds);
    let task_type_bind = match task_type {
        Some(t) => JsValue::from_str(t),
        None => JsValue::NULL,
    };

    // ── Task completion rate (filtered by task_type when given) ────────
    let tasks_row: Vec<RateRow> = db
        .prepare(
            "SELECT
               SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS numerator,
               COUNT(*) AS total
             FROM mcp_tasks
             WHERE tenant_id = ?1
               AND created_at >= ?2
               AND (?3 IS NULL OR task_type = ?3)",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&cutoff),
            task_type_bind.clone(),
        ])?
        .all()
        .await?
        .results()?;
    let (task_completed, tasks_total) = tasks_row
        .first()
        .map(|r| (r.numerator, r.total))
        .unwrap_or((0, 0));

    // ── Events count (denominator for throughput) ──────────────────────
    let events_row: Vec<CountRow> = db
        .prepare(
            "SELECT COUNT(*) AS total
             FROM events_bronze
             WHERE tenant_id = ?1 AND created_at >= ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(&cutoff)])?
        .all()
        .await?
        .results()?;
    let events_total = events_row.first().map(|r| r.total).unwrap_or(0);

    // ── Human intervention rate (escalations / decisions) ──────────────
    let decisions_row: Vec<RateRow> = db
        .prepare(
            "SELECT
               SUM(CASE WHEN decision = 'escalate' THEN 1 ELSE 0 END) AS numerator,
               COUNT(*) AS total
             FROM policy_decisions
             WHERE tenant_id = ?1 AND created_at >= ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(&cutoff)])?
        .all()
        .await?
        .results()?;
    let (escalated, decisions_total) = decisions_row
        .first()
        .map(|r| (r.numerator, r.total))
        .unwrap_or((0, 0));

    // ── MTTR (fetch ordered failure+recovery events, compute in Rust) ──
    let mttr_events: Vec<EventRow> = db
        .prepare(
            "SELECT run_id, event_type, created_at
             FROM events_bronze
             WHERE tenant_id = ?1
               AND created_at >= ?2
               AND run_id IS NOT NULL
               AND event_type IN
                 ('run_failed','task_failed','error','run_completed','task_completed','recovered')
             ORDER BY run_id ASC, created_at ASC",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(&cutoff)])?
        .all()
        .await?
        .results()?;
    let mut deltas = extract_mttr_deltas_seconds(mttr_events.into_iter().filter_map(|e| {
        let run = e.run_id?;
        let ts = iso_to_epoch_seconds(&e.created_at)?;
        Some((run, e.event_type, ts))
    }));
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mttr_p50 = percentile_sorted(&deltas, 0.50);
    let mttr_p95 = percentile_sorted(&deltas, 0.95);

    // ── Assemble KPIs and null reasons ─────────────────────────────────
    let mut kpis = Kpis::default();
    let mut null_reasons: BTreeMap<&'static str, &'static str> = BTreeMap::new();

    if tasks_total > 0 {
        kpis.task_completion_rate = Some(task_completed as f64 / tasks_total as f64);
    } else {
        null_reasons.insert("task_completion_rate", "no tasks in window");
    }

    if deltas.is_empty() {
        null_reasons.insert(
            "mttr_p50_seconds",
            "no failure→recovery transitions in window",
        );
        null_reasons.insert(
            "mttr_p95_seconds",
            "no failure→recovery transitions in window",
        );
    } else {
        kpis.mttr_p50_seconds = mttr_p50;
        kpis.mttr_p95_seconds = mttr_p95;
    }

    // Context reuse rate: deliberately deferred (see issue #105 risk section
    // — no canonical hit/miss column exists yet).
    null_reasons.insert(
        "context_reuse_rate",
        "definition deferred — see issue #105 risk section",
    );

    if decisions_total > 0 {
        kpis.human_intervention_rate = Some(escalated as f64 / decisions_total as f64);
    } else {
        null_reasons.insert("human_intervention_rate", "no policy decisions in window");
    }

    if window_seconds > 0 && events_total > 0 {
        kpis.event_throughput_per_sec = Some(events_total as f64 / window_seconds as f64);
    } else if events_total == 0 {
        null_reasons.insert("event_throughput_per_sec", "no events in window");
    }

    Ok(PilotMetrics {
        window: window_raw.to_string(),
        window_seconds,
        sample_counts: SampleCounts {
            tasks: tasks_total,
            events: events_total,
            decisions: decisions_total,
        },
        kpis,
        null_reasons,
        meta: Meta {
            generated_at: now_iso(),
            tenant_id: tenant_id.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── window parser ──────────────────────────────────────────────────

    #[test]
    fn window_parses_hours_days_weeks() {
        assert_eq!(parse_window("1h").unwrap(), ("1h".into(), 3600));
        assert_eq!(parse_window("1d").unwrap(), ("1d".into(), 86_400));
        assert_eq!(parse_window("7d").unwrap(), ("7d".into(), 604_800));
        assert_eq!(parse_window("2w").unwrap(), ("2w".into(), 1_209_600));
    }

    #[test]
    fn window_24h_equals_1d_in_seconds() {
        let (_, a) = parse_window("24h").unwrap();
        let (_, b) = parse_window("1d").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn window_rejects_empty_or_zero_or_unknown_unit() {
        assert!(parse_window("").is_err());
        assert!(parse_window("0d").is_err());
        assert!(parse_window("-1d").is_err());
        assert!(parse_window("5m").is_err()); // minutes not supported
        assert!(parse_window("abc").is_err());
    }

    // ── percentile ─────────────────────────────────────────────────────

    #[test]
    fn percentile_empty_is_none() {
        assert!(percentile_sorted(&[], 0.5).is_none());
    }

    #[test]
    fn percentile_single_value() {
        assert_eq!(percentile_sorted(&[42.0], 0.5), Some(42.0));
        assert_eq!(percentile_sorted(&[42.0], 0.95), Some(42.0));
    }

    #[test]
    fn percentile_known_quantiles() {
        // Numpy default (linear): p50 of 1..=5 is 3.0, p95 is 4.8
        let xs = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile_sorted(&xs, 0.5), Some(3.0));
        assert!((percentile_sorted(&xs, 0.95).unwrap() - 4.8).abs() < 1e-9);
        assert_eq!(percentile_sorted(&xs, 0.0), Some(1.0));
        assert_eq!(percentile_sorted(&xs, 1.0), Some(5.0));
    }

    // ── MTTR delta extraction ──────────────────────────────────────────

    #[test]
    fn mttr_pairs_failure_to_next_recovery_same_run() {
        let events = vec![
            ("run-a".to_string(), "run_failed".to_string(), 100.0),
            ("run-a".to_string(), "run_completed".to_string(), 300.0),
        ];
        assert_eq!(extract_mttr_deltas_seconds(events), vec![200.0]);
    }

    #[test]
    fn mttr_ignores_recovery_without_prior_failure() {
        let events = vec![
            ("run-a".to_string(), "run_completed".to_string(), 50.0),
            ("run-a".to_string(), "run_failed".to_string(), 100.0),
            ("run-a".to_string(), "run_completed".to_string(), 200.0),
        ];
        // The lone recovery at t=50 is ignored; the failure at t=100 closes
        // out at t=200 → one delta of 100.
        assert_eq!(extract_mttr_deltas_seconds(events), vec![100.0]);
    }

    #[test]
    fn mttr_does_not_cross_run_boundaries() {
        // A failure in run-a should NOT be closed by a recovery in run-b.
        let events = vec![
            ("run-a".to_string(), "run_failed".to_string(), 100.0),
            ("run-b".to_string(), "run_completed".to_string(), 200.0),
        ];
        assert!(extract_mttr_deltas_seconds(events).is_empty());
    }

    #[test]
    fn mttr_handles_multiple_intervals_per_run() {
        let events = vec![
            ("r".to_string(), "task_failed".to_string(), 10.0),
            ("r".to_string(), "task_completed".to_string(), 30.0), // delta 20
            ("r".to_string(), "task_failed".to_string(), 50.0),
            ("r".to_string(), "task_completed".to_string(), 80.0), // delta 30
        ];
        assert_eq!(extract_mttr_deltas_seconds(events), vec![20.0, 30.0]);
    }

    #[test]
    fn mttr_ignores_unknown_event_types() {
        let events = vec![
            ("r".to_string(), "noise".to_string(), 5.0),
            ("r".to_string(), "run_failed".to_string(), 10.0),
            ("r".to_string(), "checkpoint_written".to_string(), 15.0),
            ("r".to_string(), "run_completed".to_string(), 25.0),
        ];
        assert_eq!(extract_mttr_deltas_seconds(events), vec![15.0]);
    }

    #[test]
    fn mttr_open_failure_with_no_recovery_emits_no_delta() {
        let events = vec![("r".to_string(), "run_failed".to_string(), 10.0)];
        assert!(extract_mttr_deltas_seconds(events).is_empty());
    }
}
