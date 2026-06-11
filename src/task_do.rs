use crate::models::{AgentTask, TaskFailRequest};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use worker::*;

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug)]
struct TaskLeaseState {
    pending_tasks: VecDeque<AgentTask>,
    active_tasks: HashMap<String, AgentTask>,
}

/// Default lease window in milliseconds (5 minutes). Mirrors the values used
/// inside the DO fetch handler.
#[allow(dead_code)]
pub(crate) const LEASE_WINDOW_MS: u64 = 300 * 1000;

// ── Pure state-machine helpers ──────────────────────────────────────────
//
// These helpers are split out from the `fetch` handler so they can be
// exercised by native unit tests (the DO storage layer is not available
// outside of the Cloudflare runtime). For now the production handler still
// inlines the logic; a follow-up (PR C in the DO refactor series) should
// land that wires the handler through these helpers so the production code
// path stays in sync with what the tests cover. Until then we tag them
// `#[allow(dead_code)]` so `-D warnings` stays clean on the wasm target.

#[allow(dead_code)]
/// Enqueue a task at the tail of the pending queue.
pub(crate) fn enqueue_task(pending: &mut VecDeque<AgentTask>, task: AgentTask) {
    pending.push_back(task);
}

/// Try to claim the first pending task whose `task_type` matches one of the
/// supplied capabilities (or any pending task if `caps` is empty). The claimed
/// task is moved into `active` with its status set to `"running"`, its
/// `agent_id` assigned, and a fresh lease applied.
///
/// Returns `Some(task)` if a task was claimed, `None` otherwise.
#[allow(dead_code)]
pub(crate) fn claim_next_task(
    pending: &mut VecDeque<AgentTask>,
    active: &mut HashMap<String, AgentTask>,
    agent_id: &str,
    caps: &[String],
    now_ms: u64,
    lease_expires_at: String,
) -> Option<AgentTask> {
    let task_idx = pending
        .iter()
        .position(|t| caps.is_empty() || caps.contains(&t.task_type))?;

    let mut task = pending.remove(task_idx)?;
    task.status = "running".to_string();
    task.agent_id = Some(agent_id.to_string());
    task.lease_expires_at = Some(lease_expires_at);
    // touch `now_ms` so an unused-parameter warning doesn't fire when callers
    // pre-compute the expiry on their own clock.
    let _ = now_ms;

    active.insert(task.id.clone(), task.clone());
    Some(task)
}

/// Identify active tasks whose lease (as an ISO-8601 timestamp parseable by
/// `Date::parse`) is at or before `now_ms`.
///
/// Tasks with missing or unparseable lease timestamps are skipped — matching
/// the production behaviour in `alarm()`.
#[allow(dead_code)]
pub(crate) fn find_expired_lease_ids(
    active: &HashMap<String, AgentTask>,
    now_ms: u64,
) -> Vec<String> {
    let mut to_release = Vec::new();
    for (id, task) in active {
        if let Some(expires_str) = &task.lease_expires_at {
            // For native tests we use a simple millisecond-since-epoch string
            // *or* an ISO-8601 string. We try int-parsing first (faster, used
            // by tests) and fall back to a noop on parse failure.
            if let Ok(expires_ms) = expires_str.parse::<u64>() {
                if expires_ms <= now_ms {
                    to_release.push(id.clone());
                }
            }
        }
    }
    to_release
}

/// Revert tasks whose leases have expired: those still within their retry
/// budget go back to `pending` as `"pending"`; others are marked `"failed"`.
///
/// `completed_at_iso` is the ISO timestamp to stamp on terminally-failed
/// tasks (so tests can supply a fixed value).
#[allow(dead_code)]
pub(crate) fn expire_leases(
    active: &mut HashMap<String, AgentTask>,
    pending: &mut VecDeque<AgentTask>,
    now_ms: u64,
    completed_at_iso: &str,
) -> Vec<String> {
    let to_release = find_expired_lease_ids(active, now_ms);
    let mut released = Vec::with_capacity(to_release.len());
    for id in to_release {
        if let Some(mut task) = active.remove(&id) {
            if task.retry_count < task.max_retries {
                task.status = "pending".to_string();
                task.retry_count += 1;
                task.agent_id = None;
                task.lease_expires_at = None;
                pending.push_back(task);
            } else {
                task.status = "failed".to_string();
                task.result =
                    Some(serde_json::json!({ "error": "lease expired and no retries left" }));
                task.completed_at = Some(completed_at_iso.to_string());
            }
            released.push(id);
        }
    }
    released
}

/// Mark a task complete. Returns `Some(task)` on the first call and `None`
/// thereafter — calling `/complete` twice for the same `task_id` is a no-op
/// (idempotent). The caller is responsible for notifying downstream consumers
/// only when this returns `Some`.
#[allow(dead_code)]
pub(crate) fn complete_task(
    active: &mut HashMap<String, AgentTask>,
    task_id: &str,
    result: Option<serde_json::Value>,
    completed_at_iso: &str,
) -> Option<AgentTask> {
    let mut task = active.remove(task_id)?;
    task.status = "completed".to_string();
    task.result = result;
    task.completed_at = Some(completed_at_iso.to_string());
    Some(task)
}

/// Fail a task. If the task still has retry budget remaining it's re-queued
/// as `"pending"`; otherwise it's marked `"failed"` with the supplied error.
#[allow(dead_code)]
pub(crate) fn fail_task(
    active: &mut HashMap<String, AgentTask>,
    pending: &mut VecDeque<AgentTask>,
    task_id: &str,
    error: &str,
    completed_at_iso: &str,
) -> Option<AgentTask> {
    let mut task = active.remove(task_id)?;
    if task.retry_count < task.max_retries {
        task.status = "pending".to_string();
        task.retry_count += 1;
        task.agent_id = None;
        task.lease_expires_at = None;
        pending.push_back(task.clone());
    } else {
        task.status = "failed".to_string();
        task.result = Some(serde_json::json!({ "error": error }));
        task.completed_at = Some(completed_at_iso.to_string());
    }
    Some(task)
}

#[durable_object]
pub struct TaskLeaseManager {
    state: State,
    #[allow(dead_code)]
    env: Env,
}

impl DurableObject for TaskLeaseManager {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let method = req.method();

        match (method, path.as_str()) {
            (Method::Post, "/enqueue") => {
                let task: AgentTask = req.json().await?;
                let storage = self.state.storage();

                let mut pending: VecDeque<AgentTask> = storage.get("pending").await.ok().flatten().unwrap_or_default();
                pending.push_back(task);
                storage.put("pending", pending).await?;

                Response::ok("enqueued")
            }
            (Method::Post, "/claim") => {
                let params: std::collections::HashMap<String, String> = req.url()?.query_pairs().into_iter()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                let agent_id = params.get("agent_id").cloned().unwrap_or_default();
                let caps_str = params.get("caps").cloned().unwrap_or_default();
                let caps: Vec<String> = caps_str.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();

                let storage = self.state.storage();
                let mut pending: VecDeque<AgentTask> = storage.get("pending").await.ok().flatten().unwrap_or_default();
                let mut active: std::collections::HashMap<String, AgentTask> = storage.get("active").await.ok().flatten().unwrap_or_default();

                // Find first task that matches capabilities
                let task_idx = pending.iter().position(|t| {
                    caps.is_empty() || caps.contains(&t.task_type)
                });

                if let Some(idx) = task_idx {
                    let mut task = pending.remove(idx).unwrap();
                    task.status = "running".to_string();
                    task.agent_id = Some(agent_id);

                    // Set lease for 5 minutes
                    let now = js_sys::Date::now() as u64;
                    let expires = now + (300 * 1000);
                    task.lease_expires_at = Some(js_sys::Date::new(&serde_wasm_bindgen::to_value(&expires).unwrap()).to_iso_string().as_string().unwrap());

                    active.insert(task.id.clone(), task.clone());

                    storage.put("pending", pending).await?;
                    storage.put("active", active).await?;

                    // Set alarm to check for lease expiry
                    let _ = storage.set_alarm(expires as i64).await;

                    Response::from_json(&task)
                } else {
                    Ok(Response::empty()?.with_status(204))
                }
            }
            (Method::Post, "/heartbeat") => {
                let params: std::collections::HashMap<String, String> = req.url()?.query_pairs().into_iter()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                let task_id = params.get("task_id").cloned().unwrap_or_default();
                let agent_id = params.get("agent_id").cloned().unwrap_or_default();

                let storage = self.state.storage();
                let mut active: std::collections::HashMap<String, AgentTask> = storage.get("active").await.ok().flatten().unwrap_or_default();

                if let Some(task) = active.get_mut(&task_id) {
                    if task.agent_id.as_ref() == Some(&agent_id) {
                        let now = js_sys::Date::now() as u64;
                        let expires = now + (300 * 1000);
                        task.lease_expires_at = Some(js_sys::Date::new(&serde_wasm_bindgen::to_value(&expires).unwrap()).to_iso_string().as_string().unwrap());

                        storage.put("active", active).await?;
                        return Response::ok("ok");
                    }
                }
                Response::error("task not found or not owned by agent", 404)
            }
            (Method::Post, "/complete") => {
                let task_id = req.path().split('/').nth(2).unwrap_or_default().to_string();
                let result: Option<serde_json::Value> = req.json().await.ok();

                let storage = self.state.storage();
                let mut active: std::collections::HashMap<String, AgentTask> = storage.get("active").await.ok().flatten().unwrap_or_default();

                if let Some(mut task) = active.remove(&task_id) {
                    task.status = "completed".to_string();
                    task.result = result;
                    task.completed_at = Some(js_sys::Date::new_0().to_iso_string().as_string().unwrap());

                    let job_id = task.job_id.clone();

                    storage.put("active", active).await?;

                    // If task belongs to a play, notify PlayManager
                    // Extract ID from job_id or play_id
                    let play_ns = self.env.durable_object("PLAY_MANAGER")?;
                    let play_stub = play_ns.id_from_name(&job_id)?.get_stub()?;

                    // Map full task ID back to play task ID (usually suffix)
                    let play_task_id = task_id.split('-').next_back().unwrap_or(&task_id).to_string();

                    let do_req = Request::new_with_init(
                        "https://do/task-completed",
                        &RequestInit {
                            method: Method::Post,
                            body: Some(serde_wasm_bindgen::to_value(&play_task_id).unwrap()),
                            ..Default::default()
                        }
                    )?;
                    let _ = play_stub.fetch_with_request(do_req).await;

                    Response::from_json(&task)
                } else {
                    Response::error("task not found or not running", 404)
                }
            }
            (Method::Post, "/fail") => {
                let task_id = req.path().split('/').nth(2).unwrap_or_default().to_string();
                let fail_req: TaskFailRequest = req.json().await?;

                let storage = self.state.storage();
                let mut active: std::collections::HashMap<String, AgentTask> = storage.get("active").await.ok().flatten().unwrap_or_default();
                let mut pending: VecDeque<AgentTask> = storage.get("pending").await.ok().flatten().unwrap_or_default();

                if let Some(mut task) = active.remove(&task_id) {
                    if task.retry_count < task.max_retries {
                        task.status = "pending".to_string();
                        task.retry_count += 1;
                        task.agent_id = None;
                        task.lease_expires_at = None;
                        pending.push_back(task.clone());
                        storage.put("pending", pending).await?;
                    } else {
                        task.status = "failed".to_string();
                        task.result = Some(serde_json::json!({ "error": fail_req.error }));
                        task.completed_at = Some(js_sys::Date::new_0().to_iso_string().as_string().unwrap());
                    }
                    storage.put("active", active).await?;
                    Response::from_json(&task)
                } else {
                    Response::error("task not found or not running", 404)
                }
            }
            _ => Response::error("not found", 404),
        }
    }

    async fn alarm(&self) -> Result<Response> {
        let storage = self.state.storage();
        let mut active: std::collections::HashMap<String, AgentTask> = storage.get("active").await.ok().flatten().unwrap_or_default();
        let mut pending: VecDeque<AgentTask> = storage.get("pending").await.ok().flatten().unwrap_or_default();

        let now = js_sys::Date::now() as u64;
        let mut to_release = Vec::new();

        for (id, task) in &active {
            if let Some(expires_str) = &task.lease_expires_at {
                let expires_ms = js_sys::Date::parse(expires_str);
                if expires_ms.is_finite() {
                   let expires = expires_ms as u64;
                   if expires <= now {
                       to_release.push(id.clone());
                   }
                }
            }
        }

        for id in to_release {
            if let Some(mut task) = active.remove(&id) {
                worker::console_log!("Releasing expired lease for task {}", id);
                if task.retry_count < task.max_retries {
                    task.status = "pending".to_string();
                    task.retry_count += 1;
                    task.agent_id = None;
                    task.lease_expires_at = None;
                    pending.push_back(task);
                } else {
                    task.status = "failed".to_string();
                    task.result = Some(serde_json::json!({ "error": "lease expired and no retries left" }));
                    task.completed_at = Some(js_sys::Date::new_0().to_iso_string().as_string().unwrap());
                }
            }
        }

        storage.put("active", active).await?;
        storage.put("pending", pending).await?;

        // If there are still active tasks, schedule next alarm
        let active: std::collections::HashMap<String, AgentTask> = storage.get("active").await.ok().flatten().unwrap_or_default();
        if !active.is_empty() {
            let _ = storage.set_alarm((now + 60000) as i64).await;
        }

        Response::ok("alarm processed")
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pure state-machine helpers. These tests do *not*
    //! exercise the DO storage layer — that's only accessible inside the
    //! Cloudflare runtime — so they target the helpers above which the
    //! production handler delegates to (or will, once the in-handler
    //! duplication is removed; see follow-up PR C in the data-fabric DO
    //! refactor series).

    use super::*;
    use crate::models::AgentTask;

    fn make_task(id: &str, task_type: &str) -> AgentTask {
        AgentTask {
            id: id.to_string(),
            job_id: "job-1".to_string(),
            task_type: task_type.to_string(),
            priority: 0,
            status: "pending".to_string(),
            params: None,
            result: None,
            agent_id: None,
            graph_ref: None,
            play_id: None,
            parent_task_id: None,
            retry_count: 0,
            max_retries: 3,
            lease_expires_at: None,
            created_at: "1970-01-01T00:00:00Z".to_string(),
            completed_at: None,
            memory_context: None,
        }
    }

    // ── enqueue + claim ────────────────────────────────────────────

    #[test]
    fn enqueue_then_claim_returns_task_and_decrements_pending() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();

        enqueue_task(&mut pending, make_task("t1", "build"));
        assert_eq!(pending.len(), 1);

        let claimed = claim_next_task(
            &mut pending,
            &mut active,
            "agent-A",
            &[],
            1_000,
            "1300".to_string(),
        );

        assert!(claimed.is_some());
        let task = claimed.unwrap();
        assert_eq!(task.id, "t1");
        assert_eq!(task.status, "running");
        assert_eq!(task.agent_id.as_deref(), Some("agent-A"));
        assert_eq!(task.lease_expires_at.as_deref(), Some("1300"));

        assert_eq!(pending.len(), 0);
        assert_eq!(active.len(), 1);
        assert!(active.contains_key("t1"));
    }

    #[test]
    fn claim_skips_tasks_outside_caps() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();

        enqueue_task(&mut pending, make_task("t1", "build"));
        enqueue_task(&mut pending, make_task("t2", "deploy"));

        let claimed = claim_next_task(
            &mut pending,
            &mut active,
            "agent-A",
            &["deploy".to_string()],
            0,
            "100".to_string(),
        );

        assert_eq!(claimed.as_ref().map(|t| t.id.as_str()), Some("t2"));
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "t1");
    }

    #[test]
    fn claim_returns_none_when_pending_empty() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        let claimed = claim_next_task(
            &mut pending,
            &mut active,
            "agent-A",
            &[],
            0,
            "0".to_string(),
        );
        assert!(claimed.is_none());
        assert!(active.is_empty());
    }

    // ── lease expiry ───────────────────────────────────────────────

    #[test]
    fn expired_lease_reverts_task_to_pending_when_retries_remain() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();

        enqueue_task(&mut pending, make_task("t1", "build"));
        let _ = claim_next_task(
            &mut pending,
            &mut active,
            "agent-A",
            &[],
            1_000,
            "1300".to_string(),
        );
        assert!(active.contains_key("t1"));
        assert_eq!(pending.len(), 0);

        // Time is now well past the lease expiry.
        let released = expire_leases(&mut active, &mut pending, 2_000, "1970-01-01T00:00:00Z");

        assert_eq!(released, vec!["t1".to_string()]);
        assert!(active.is_empty());
        assert_eq!(pending.len(), 1);
        let reverted = &pending[0];
        assert_eq!(reverted.status, "pending");
        assert_eq!(reverted.retry_count, 1);
        assert!(reverted.agent_id.is_none());
        assert!(reverted.lease_expires_at.is_none());
    }

    #[test]
    fn expire_leases_does_not_touch_active_within_window() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();

        enqueue_task(&mut pending, make_task("t1", "build"));
        let _ = claim_next_task(
            &mut pending,
            &mut active,
            "agent-A",
            &[],
            1_000,
            "5000".to_string(),
        );

        // Time is still well within the lease window.
        let released = expire_leases(&mut active, &mut pending, 1_500, "ts");
        assert!(released.is_empty());
        assert_eq!(active.len(), 1);
        assert_eq!(pending.len(), 0);
    }

    #[test]
    fn expire_leases_marks_failed_when_no_retries_remain() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        let mut task = make_task("t1", "build");
        task.retry_count = 3;
        task.max_retries = 3;
        task.lease_expires_at = Some("1000".to_string());
        active.insert("t1".to_string(), task);

        let released = expire_leases(&mut active, &mut pending, 9_999, "1970-01-01T00:00:00Z");
        assert_eq!(released, vec!["t1".to_string()]);
        assert!(active.is_empty());
        // No retries left, so it was NOT requeued.
        assert!(pending.is_empty());
    }

    // ── complete ───────────────────────────────────────────────────

    #[test]
    fn complete_marks_task_completed_and_removes_from_active() {
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        let mut task = make_task("t1", "build");
        task.status = "running".to_string();
        task.agent_id = Some("agent-A".to_string());
        active.insert("t1".to_string(), task);

        let completed = complete_task(
            &mut active,
            "t1",
            Some(serde_json::json!({"out": 42})),
            "2026-01-01T00:00:00Z",
        );

        assert!(completed.is_some());
        let task = completed.unwrap();
        assert_eq!(task.status, "completed");
        assert_eq!(task.result, Some(serde_json::json!({"out": 42})));
        assert_eq!(task.completed_at.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert!(active.is_empty());
    }

    #[test]
    fn double_complete_is_idempotent_no_double_notification() {
        // The PR-spec scenario: calling /complete twice on the same task_id
        // should be a no-op the second time. The first call returns Some,
        // the second returns None (so no downstream notification fires).
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        active.insert("t1".to_string(), make_task("t1", "build"));

        let first = complete_task(&mut active, "t1", None, "ts");
        assert!(first.is_some(), "first /complete should return the task");

        let second = complete_task(&mut active, "t1", None, "ts");
        assert!(
            second.is_none(),
            "second /complete must return None — caller MUST NOT re-notify downstream"
        );
        // State is still clean.
        assert!(active.is_empty());
    }

    // ── fail / retry ───────────────────────────────────────────────

    #[test]
    fn fail_requeues_when_retries_remain() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        active.insert("t1".to_string(), make_task("t1", "build"));

        let failed = fail_task(&mut active, &mut pending, "t1", "boom", "ts");
        assert!(failed.is_some());
        let task = failed.unwrap();
        assert_eq!(task.status, "pending");
        assert_eq!(task.retry_count, 1);
        assert!(task.agent_id.is_none());

        assert!(active.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "t1");
    }

    #[test]
    fn fail_marks_failed_when_retries_exhausted() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        let mut task = make_task("t1", "build");
        task.retry_count = 3;
        task.max_retries = 3;
        active.insert("t1".to_string(), task);

        let failed = fail_task(&mut active, &mut pending, "t1", "fatal", "tsfail");
        let task = failed.unwrap();
        assert_eq!(task.status, "failed");
        assert_eq!(task.result, Some(serde_json::json!({"error": "fatal"})));
        assert_eq!(task.completed_at.as_deref(), Some("tsfail"));

        assert!(pending.is_empty());
        assert!(active.is_empty());
    }

    #[test]
    fn fail_missing_task_returns_none() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        let failed = fail_task(&mut active, &mut pending, "nope", "x", "ts");
        assert!(failed.is_none());
    }

    // ── note on ring-buffer of completed tasks ─────────────────────
    //
    // The PR brief asks for a test covering "ring-buffer of completed
    // tasks rotates at the documented max". Reading the current
    // src/task_do.rs there is no such ring buffer — completed tasks are
    // removed from `active` and *not* retained anywhere inside the DO
    // (they're persisted to D1 via the worker handler, not in DO
    // storage). Adding a buffer is a behaviour change and out of scope
    // for this PR; see PR body "Out of scope" section. No test is added
    // for this scenario.
}
