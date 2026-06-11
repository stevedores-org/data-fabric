use crate::models::{AgentTask, TaskFailRequest};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use worker::*;

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug)]
struct TaskLeaseState {
    pending_tasks: VecDeque<AgentTask>,
    active_tasks: std::collections::HashMap<String, AgentTask>,
}

/// Maximum number of pending tasks held in a single TaskLeaseManager DO.
///
/// Chosen to bound DO storage growth when producers outrun consumers. With an
/// average serialized `AgentTask` of ~2 KB, 10k pending tasks corresponds to
/// roughly 20 MB of DO storage — well under the per-DO storage limits but
/// enough to absorb a normal burst. Tunable; revisit if real workloads
/// regularly hit the cap.
pub const MAX_PENDING_TASKS: usize = 10_000;

/// Storage key for the cumulative count of enqueue requests rejected due to
/// backpressure. Surfaced later via a `/metrics` endpoint (not in this PR).
pub const PENDING_REJECTED_TOTAL_KEY: &str = "pending_rejected_total";

/// Retry-After value (seconds) returned alongside 429 from `/enqueue` when the
/// queue is at capacity. 30s gives consumers time to drain a meaningful chunk
/// without leaving producers idle for too long.
pub const ENQUEUE_RETRY_AFTER_SECS: u32 = 30;

/// Pure decision: should this enqueue be accepted, given the current queue
/// length and the configured maximum? Extracted so the backpressure rule can
/// be unit-tested without the wasm runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueDecision {
    Accept,
    Reject,
}

pub fn enqueue_decision(current_len: usize, max: usize) -> EnqueueDecision {
    if current_len >= max {
        EnqueueDecision::Reject
    } else {
        EnqueueDecision::Accept
    }
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

                // Backpressure: reject before deserialization-heavy work piles
                // up. Counter persists across requests so operators can see
                // sustained pressure via a future /metrics endpoint.
                if enqueue_decision(pending.len(), MAX_PENDING_TASKS)
                    == EnqueueDecision::Reject
                {
                    let prev_rejected: u64 = storage
                        .get(PENDING_REJECTED_TOTAL_KEY)
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or(0);
                    let next_rejected = prev_rejected.saturating_add(1);
                    storage.put(PENDING_REJECTED_TOTAL_KEY, next_rejected).await?;

                    let headers = Headers::new();
                    headers.set("retry-after", &ENQUEUE_RETRY_AFTER_SECS.to_string())?;
                    headers.set("content-type", "application/json")?;
                    let body = serde_json::json!({
                        "error": {
                            "code": "QUEUE_FULL",
                            "message": "pending_tasks queue at capacity; retry after the indicated delay",
                            "details": {
                                "max_pending_tasks": MAX_PENDING_TASKS,
                                "retry_after_seconds": ENQUEUE_RETRY_AFTER_SECS,
                            }
                        }
                    });
                    return Ok(Response::from_json(&body)?
                        .with_status(429)
                        .with_headers(headers));
                }

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
    use super::*;

    // ── enqueue_decision (backpressure rule) ────────────────────

    #[test]
    fn enqueue_decision_accepts_when_below_capacity() {
        assert_eq!(enqueue_decision(0, 10), EnqueueDecision::Accept);
        assert_eq!(enqueue_decision(5, 10), EnqueueDecision::Accept);
        assert_eq!(enqueue_decision(9, 10), EnqueueDecision::Accept);
    }

    #[test]
    fn enqueue_decision_rejects_at_capacity() {
        // The handler increments the rejection counter and returns 429
        // whenever this returns `Reject`, so the boundary behavior is part
        // of the public contract.
        assert_eq!(enqueue_decision(10, 10), EnqueueDecision::Reject);
        assert_eq!(enqueue_decision(11, 10), EnqueueDecision::Reject);
        assert_eq!(enqueue_decision(usize::MAX, 10), EnqueueDecision::Reject);
    }

    #[test]
    fn enqueue_decision_uses_configured_max_pending_tasks() {
        // Capacity boundary using the live constant: simulating a queue at
        // MAX_PENDING_TASKS - 1 still accepts, MAX_PENDING_TASKS rejects.
        assert_eq!(
            enqueue_decision(MAX_PENDING_TASKS - 1, MAX_PENDING_TASKS),
            EnqueueDecision::Accept,
        );
        assert_eq!(
            enqueue_decision(MAX_PENDING_TASKS, MAX_PENDING_TASKS),
            EnqueueDecision::Reject,
        );
    }

    /// Simulates the rejection-counter bookkeeping the `/enqueue` handler
    /// performs on a rejected enqueue: read prior total, increment, persist.
    fn simulate_rejection_counter(prev: u64) -> u64 {
        prev.saturating_add(1)
    }

    #[test]
    fn rejection_counter_increments_per_rejected_enqueue() {
        let mut total: u64 = 0;
        for expected in 1..=5u64 {
            // Each call to enqueue at capacity triggers exactly one
            // increment, matching the handler's behavior.
            assert_eq!(
                enqueue_decision(MAX_PENDING_TASKS, MAX_PENDING_TASKS),
                EnqueueDecision::Reject,
            );
            total = simulate_rejection_counter(total);
            assert_eq!(total, expected);
        }
    }

    #[test]
    fn rejection_counter_saturates_instead_of_wrapping() {
        // Defensive: the handler uses `saturating_add` so a long-lived DO
        // can't underflow the metric back to zero.
        assert_eq!(simulate_rejection_counter(u64::MAX), u64::MAX);
    }

    #[test]
    fn max_pending_tasks_constant_is_documented_value() {
        // Tripwire: changing the cap is a deliberate ops decision; the test
        // forces the changer to update the doc comment and this assertion
        // together.
        assert_eq!(MAX_PENDING_TASKS, 10_000);
    }

    #[test]
    fn retry_after_is_documented_value() {
        assert_eq!(ENQUEUE_RETRY_AFTER_SECS, 30);
    }
}
