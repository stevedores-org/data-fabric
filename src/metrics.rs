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
//!
//! Module shape is deliberately split: `query_inputs` does D1 IO and returns
//! a plain `PilotInputs`; `assemble` is pure and constructs the response.
//! The split lets `assemble` carry the null/value branch logic under direct
//! unit-test coverage.

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

// ── Wire types ─────────────────────────────────────────────────────────

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

// ── Intermediate (testable) shape ──────────────────────────────────────

/// Numerator/denominator pair. Conventionally `denominator = 0` ⇒ no data.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CountTotal {
    pub numerator: i64,
    pub denominator: i64,
}

impl CountTotal {
    /// `numerator / denominator` when `denominator > 0`, else `None`.
    pub fn rate(self) -> Option<f64> {
        if self.denominator > 0 {
            Some(self.numerator as f64 / self.denominator as f64)
        } else {
            None
        }
    }
}

/// Everything `assemble` needs to construct a `PilotMetrics`. Produced by
/// `query_inputs` in production, hand-built in tests.
#[derive(Debug, Default, Clone)]
pub struct PilotInputs {
    pub task_completion: CountTotal,
    pub events_total: i64,
    pub human_intervention: CountTotal,
    pub context_reuse: CountTotal,
    /// Unsorted; `assemble` sorts before percentile.
    pub mttr_deltas_seconds: Vec<f64>,
}

// ── Pure helpers ───────────────────────────────────────────────────────

/// Parse `?window=` to canonical `(echoed_string, seconds)`. Accepts `Nh`,
/// `Nd`, `Nw` for hours, days, weeks. `24h` and `1d` resolve to the same
/// `seconds` deliberately. UTF-8 safe — `?window=1🌒` returns Err rather
/// than panicking on a byte-boundary slice.
pub fn parse_window(raw: &str) -> Result<(String, i64)> {
    let unit_char = raw
        .chars()
        .last()
        .ok_or_else(|| Error::RustError("window must not be empty".into()))?;
    let num_part = &raw[..raw.len() - unit_char.len_utf8()];
    let n: i64 = num_part
        .parse()
        .map_err(|_| Error::RustError(format!("window: bad number in {raw:?}")))?;
    if n <= 0 {
        return Err(Error::RustError(format!(
            "window: must be positive, got {n}"
        )));
    }
    let secs_per_unit: i64 = match unit_char {
        'h' => 3600,
        'd' => 86_400,
        'w' => 604_800,
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

/// Build the `PilotMetrics` from already-fetched counts and deltas.
///
/// Pure — does not touch D1 or the clock. The clock value is passed in so
/// tests can pin `generated_at`.
pub fn assemble(
    window_raw: &str,
    window_seconds: i64,
    tenant_id: &str,
    inputs: PilotInputs,
    generated_at: String,
) -> PilotMetrics {
    let PilotInputs {
        task_completion,
        events_total,
        human_intervention,
        context_reuse,
        mttr_deltas_seconds,
    } = inputs;

    let mut deltas = mttr_deltas_seconds;
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut kpis = Kpis::default();
    let mut null_reasons: BTreeMap<&'static str, &'static str> = BTreeMap::new();

    match task_completion.rate() {
        Some(r) => kpis.task_completion_rate = Some(r),
        None => {
            null_reasons.insert("task_completion_rate", "no tasks in window");
        }
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
        kpis.mttr_p50_seconds = percentile_sorted(&deltas, 0.50);
        kpis.mttr_p95_seconds = percentile_sorted(&deltas, 0.95);
    }

    match context_reuse.rate() {
        Some(r) => kpis.context_reuse_rate = Some(r),
        None => {
            null_reasons.insert("context_reuse_rate", "no checkpoint_read events in window");
        }
    }

    match human_intervention.rate() {
        Some(r) => kpis.human_intervention_rate = Some(r),
        None => {
            null_reasons.insert("human_intervention_rate", "no policy decisions in window");
        }
    }

    if window_seconds > 0 && events_total > 0 {
        kpis.event_throughput_per_sec = Some(events_total as f64 / window_seconds as f64);
    } else {
        null_reasons.insert("event_throughput_per_sec", "no events in window");
    }

    PilotMetrics {
        window: window_raw.to_string(),
        window_seconds,
        sample_counts: SampleCounts {
            tasks: task_completion.denominator,
            events: events_total,
            decisions: human_intervention.denominator,
        },
        kpis,
        null_reasons,
        meta: Meta {
            generated_at,
            tenant_id: tenant_id.to_string(),
        },
    }
}

// ── D1 query layer ─────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct RateRow {
    numerator: Option<i64>,
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

impl From<&RateRow> for CountTotal {
    fn from(r: &RateRow) -> Self {
        Self {
            numerator: r.numerator.unwrap_or(0),
            denominator: r.total,
        }
    }
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

/// Cap on rows fetched for the MTTR computation. Each row is small (~80
/// bytes) so this is well under D1's response-size cap, but a runaway
/// tenant could otherwise OOM the Worker. When the cap is hit the reported
/// p50/p95 reflect only the lexicographically-first
/// `MTTR_EVENT_FETCH_LIMIT` rows after sorting by `(run_id, created_at)` —
/// approximate at high volumes. See `docs/ws10/METRICS_ENDPOINT.md`.
const MTTR_EVENT_FETCH_LIMIT: i64 = 50_000;

/// Run the five D1 queries that feed `assemble`. Returns the intermediate
/// shape so tests can exercise the assembly logic without a D1 binding.
///
/// The five queries don't depend on each other, so they're polled
/// concurrently via `try_join!`. Workers are single-threaded but D1 I/O
/// interleaves, turning ~5 × RTT into ~1 × RTT.
pub async fn query_inputs(
    db: &D1Database,
    tenant_id: &str,
    cutoff_iso: &str,
    task_type: Option<&str>,
) -> Result<PilotInputs> {
    let (task_completion, events_total, human_intervention, context_reuse, mttr_events) = futures_util::try_join!(
        fetch_task_completion(db, tenant_id, cutoff_iso, task_type),
        fetch_events_total(db, tenant_id, cutoff_iso),
        fetch_human_intervention(db, tenant_id, cutoff_iso),
        fetch_context_reuse(db, tenant_id, cutoff_iso),
        fetch_mttr_events(db, tenant_id, cutoff_iso),
    )?;

    let mttr_deltas_seconds =
        extract_mttr_deltas_seconds(mttr_events.into_iter().filter_map(|e| {
            let run = e.run_id?;
            let ts = iso_to_epoch_seconds(&e.created_at)?;
            Some((run, e.event_type, ts))
        }));

    Ok(PilotInputs {
        task_completion,
        events_total,
        human_intervention,
        context_reuse,
        mttr_deltas_seconds,
    })
}

async fn fetch_task_completion(
    db: &D1Database,
    tenant_id: &str,
    cutoff_iso: &str,
    task_type: Option<&str>,
) -> Result<CountTotal> {
    let task_type_bind = match task_type {
        Some(t) => JsValue::from_str(t),
        None => JsValue::NULL,
    };
    let rows: Vec<RateRow> = db
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
            JsValue::from_str(cutoff_iso),
            task_type_bind,
        ])?
        .all()
        .await?
        .results()?;
    Ok(rows.first().map(CountTotal::from).unwrap_or_default())
}

async fn fetch_events_total(db: &D1Database, tenant_id: &str, cutoff_iso: &str) -> Result<i64> {
    let rows: Vec<CountRow> = db
        .prepare(
            "SELECT COUNT(*) AS total
             FROM events_bronze
             WHERE tenant_id = ?1 AND created_at >= ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(cutoff_iso)])?
        .all()
        .await?
        .results()?;
    Ok(rows.first().map(|r| r.total).unwrap_or(0))
}

async fn fetch_human_intervention(
    db: &D1Database,
    tenant_id: &str,
    cutoff_iso: &str,
) -> Result<CountTotal> {
    let rows: Vec<RateRow> = db
        .prepare(
            "SELECT
               SUM(CASE WHEN decision = 'escalate' THEN 1 ELSE 0 END) AS numerator,
               COUNT(*) AS total
             FROM policy_decisions
             WHERE tenant_id = ?1 AND created_at >= ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(cutoff_iso)])?
        .all()
        .await?
        .results()?;
    Ok(rows.first().map(CountTotal::from).unwrap_or_default())
}

/// Context-reuse stopgap (docs/ws10/METRICS_ENDPOINT.md):
/// among `events_bronze` rows with `event_type = 'checkpoint_read'`, count
/// those whose `payload.hit` is truthy. `json_extract` returns integer `1`
/// for a JSON boolean `true`, but producers in the wild also emit the
/// string `"true"` (various casings) — accept all common variants.
async fn fetch_context_reuse(
    db: &D1Database,
    tenant_id: &str,
    cutoff_iso: &str,
) -> Result<CountTotal> {
    let rows: Vec<RateRow> = db
        .prepare(
            "SELECT
               SUM(CASE
                 WHEN json_extract(payload, '$.hit') IN (1, 'true', 'True', 'TRUE')
                   THEN 1 ELSE 0
               END) AS numerator,
               COUNT(*) AS total
             FROM events_bronze
             WHERE tenant_id = ?1
               AND created_at >= ?2
               AND event_type = 'checkpoint_read'",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(cutoff_iso)])?
        .all()
        .await?
        .results()?;
    Ok(rows.first().map(CountTotal::from).unwrap_or_default())
}

async fn fetch_mttr_events(
    db: &D1Database,
    tenant_id: &str,
    cutoff_iso: &str,
) -> Result<Vec<EventRow>> {
    let rows: Vec<EventRow> = db
        .prepare(
            "SELECT run_id, event_type, created_at
             FROM events_bronze
             WHERE tenant_id = ?1
               AND created_at >= ?2
               AND run_id IS NOT NULL
               AND event_type IN
                 ('run_failed','task_failed','error','run_completed','task_completed','recovered')
             ORDER BY run_id ASC, created_at ASC
             LIMIT ?3",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(cutoff_iso),
            JsValue::from_f64(MTTR_EVENT_FETCH_LIMIT as f64),
        ])?
        .all()
        .await?
        .results()?;
    Ok(rows)
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
    let inputs = query_inputs(db, tenant_id, &cutoff, task_type).await?;
    Ok(assemble(
        window_raw,
        window_seconds,
        tenant_id,
        inputs,
        now_iso(),
    ))
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

    #[test]
    fn window_multibyte_unit_returns_err_not_panic() {
        // Previously: `raw.split_at(raw.len() - 1)` panicked on a non-ASCII
        // last byte. Now: chars().last() handles the boundary correctly.
        assert!(parse_window("1🌒").is_err());
        assert!(parse_window("🌒").is_err());
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

    // ── CountTotal ─────────────────────────────────────────────────────

    #[test]
    fn count_total_rate_handles_zero_and_normal() {
        assert_eq!(CountTotal::default().rate(), None);
        assert_eq!(
            CountTotal {
                numerator: 0,
                denominator: 5
            }
            .rate(),
            Some(0.0)
        );
        assert_eq!(
            CountTotal {
                numerator: 3,
                denominator: 4
            }
            .rate(),
            Some(0.75)
        );
    }

    // ── assemble: happy path ──────────────────────────────────────────

    fn inputs_for_assemble_happy() -> PilotInputs {
        PilotInputs {
            task_completion: CountTotal {
                numerator: 8,
                denominator: 10,
            },
            events_total: 86_400, // 1/sec at 1d window
            human_intervention: CountTotal {
                numerator: 2,
                denominator: 10,
            },
            context_reuse: CountTotal {
                numerator: 7,
                denominator: 20,
            },
            mttr_deltas_seconds: vec![100.0, 200.0, 300.0, 400.0, 500.0],
        }
    }

    #[test]
    fn assemble_happy_path_fills_every_kpi() {
        let m = assemble(
            "1d",
            86_400,
            "tenant-x",
            inputs_for_assemble_happy(),
            "2026-05-25T00:00:00Z".into(),
        );

        assert_eq!(m.window, "1d");
        assert_eq!(m.window_seconds, 86_400);
        assert_eq!(m.meta.tenant_id, "tenant-x");
        assert_eq!(m.meta.generated_at, "2026-05-25T00:00:00Z");

        assert_eq!(m.sample_counts.tasks, 10);
        assert_eq!(m.sample_counts.events, 86_400);
        assert_eq!(m.sample_counts.decisions, 10);

        assert_eq!(m.kpis.task_completion_rate, Some(0.8));
        assert_eq!(m.kpis.human_intervention_rate, Some(0.2));
        assert_eq!(m.kpis.context_reuse_rate, Some(0.35));
        assert_eq!(m.kpis.event_throughput_per_sec, Some(1.0));
        assert_eq!(m.kpis.mttr_p50_seconds, Some(300.0));
        assert!((m.kpis.mttr_p95_seconds.unwrap() - 480.0).abs() < 1e-9);

        assert!(
            m.null_reasons.is_empty(),
            "happy path should have no null_reasons, got {:?}",
            m.null_reasons
        );
    }

    // ── assemble: each KPI's null branch ──────────────────────────────

    #[test]
    fn assemble_marks_task_completion_null_when_no_tasks() {
        let mut inputs = inputs_for_assemble_happy();
        inputs.task_completion = CountTotal::default();
        let m = assemble("1d", 86_400, "t", inputs, "now".into());
        assert_eq!(m.kpis.task_completion_rate, None);
        assert_eq!(
            m.null_reasons.get("task_completion_rate"),
            Some(&"no tasks in window")
        );
        assert_eq!(m.sample_counts.tasks, 0);
    }

    #[test]
    fn assemble_marks_human_intervention_null_when_no_decisions() {
        let mut inputs = inputs_for_assemble_happy();
        inputs.human_intervention = CountTotal::default();
        let m = assemble("1d", 86_400, "t", inputs, "now".into());
        assert_eq!(m.kpis.human_intervention_rate, None);
        assert_eq!(
            m.null_reasons.get("human_intervention_rate"),
            Some(&"no policy decisions in window")
        );
    }

    #[test]
    fn assemble_marks_context_reuse_null_when_no_checkpoint_reads() {
        let mut inputs = inputs_for_assemble_happy();
        inputs.context_reuse = CountTotal::default();
        let m = assemble("1d", 86_400, "t", inputs, "now".into());
        assert_eq!(m.kpis.context_reuse_rate, None);
        assert_eq!(
            m.null_reasons.get("context_reuse_rate"),
            Some(&"no checkpoint_read events in window")
        );
    }

    #[test]
    fn assemble_marks_throughput_null_when_no_events() {
        let mut inputs = inputs_for_assemble_happy();
        inputs.events_total = 0;
        let m = assemble("1d", 86_400, "t", inputs, "now".into());
        assert_eq!(m.kpis.event_throughput_per_sec, None);
        assert_eq!(
            m.null_reasons.get("event_throughput_per_sec"),
            Some(&"no events in window")
        );
    }

    #[test]
    fn assemble_marks_both_mttr_null_when_no_deltas() {
        let mut inputs = inputs_for_assemble_happy();
        inputs.mttr_deltas_seconds = vec![];
        let m = assemble("1d", 86_400, "t", inputs, "now".into());
        assert_eq!(m.kpis.mttr_p50_seconds, None);
        assert_eq!(m.kpis.mttr_p95_seconds, None);
        assert_eq!(
            m.null_reasons.get("mttr_p50_seconds"),
            Some(&"no failure→recovery transitions in window")
        );
        assert_eq!(
            m.null_reasons.get("mttr_p95_seconds"),
            Some(&"no failure→recovery transitions in window")
        );
    }

    // ── assemble: edge cases ──────────────────────────────────────────

    #[test]
    fn assemble_genuine_zero_numerator_is_not_null() {
        // 0 escalations among 10 decisions → rate = 0.0, NOT null.
        let mut inputs = inputs_for_assemble_happy();
        inputs.human_intervention = CountTotal {
            numerator: 0,
            denominator: 10,
        };
        let m = assemble("1d", 86_400, "t", inputs, "now".into());
        assert_eq!(m.kpis.human_intervention_rate, Some(0.0));
        assert!(!m.null_reasons.contains_key("human_intervention_rate"));
    }

    #[test]
    fn assemble_sorts_unsorted_mttr_deltas_before_percentile() {
        let mut inputs = inputs_for_assemble_happy();
        inputs.mttr_deltas_seconds = vec![500.0, 100.0, 400.0, 200.0, 300.0];
        let m = assemble("1d", 86_400, "t", inputs, "now".into());
        // Same percentiles as the happy-path test: p50=300, p95=480.
        assert_eq!(m.kpis.mttr_p50_seconds, Some(300.0));
        assert!((m.kpis.mttr_p95_seconds.unwrap() - 480.0).abs() < 1e-9);
    }

    #[test]
    fn assemble_all_empty_marks_every_nullable_kpi() {
        let inputs = PilotInputs::default();
        let m = assemble("1d", 86_400, "t", inputs, "now".into());
        assert!(m.kpis.task_completion_rate.is_none());
        assert!(m.kpis.mttr_p50_seconds.is_none());
        assert!(m.kpis.mttr_p95_seconds.is_none());
        assert!(m.kpis.context_reuse_rate.is_none());
        assert!(m.kpis.human_intervention_rate.is_none());
        assert!(m.kpis.event_throughput_per_sec.is_none());
        // 6 distinct reasons (mttr p50 and p95 share their reason string but
        // sit under separate keys).
        assert_eq!(m.null_reasons.len(), 6);
    }

    // ── PilotMetrics serialization shape ──────────────────────────────

    #[test]
    fn serialized_shape_omits_null_reasons_when_empty() {
        let m = assemble("1d", 86_400, "t", inputs_for_assemble_happy(), "now".into());
        let json = serde_json::to_value(&m).unwrap();
        assert!(
            json.get("null_reasons").is_none(),
            "expected null_reasons omitted, got {json:?}"
        );
    }

    #[test]
    fn serialized_shape_keeps_null_reasons_when_nonempty() {
        let inputs = PilotInputs::default();
        let m = assemble("1d", 86_400, "t", inputs, "now".into());
        let json = serde_json::to_value(&m).unwrap();
        assert!(json.get("null_reasons").is_some());
    }
}
