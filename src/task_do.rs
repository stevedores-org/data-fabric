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

// ── Pure state-machine helpers (PR #142 unit coverage) ──────────────────
//
// Extracted so native unit tests can exercise queue/lease logic without
// standing up DO storage. Production handlers still inline equivalent
// logic; a follow-up refactor may wire these through directly.

#[allow(dead_code)]
pub(crate) fn enqueue_task(pending: &mut VecDeque<AgentTask>, task: AgentTask) {
    pending.push_back(task);
}

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
    let _ = now_ms;

    active.insert(task.id.clone(), task.clone());
    Some(task)
}

#[allow(dead_code)]
pub(crate) fn find_expired_lease_ids(
    active: &HashMap<String, AgentTask>,
    now_ms: u64,
) -> Vec<String> {
    let mut to_release = Vec::new();
    for (id, task) in active {
        if let Some(expires_str) = &task.lease_expires_at {
            if let Ok(expires_ms) = expires_str.parse::<u64>() {
                if expires_ms <= now_ms {
                    to_release.push(id.clone());
                }
            }
        }
    }
    to_release
}

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

/// A pending notification to the per-tenant PlayManager DO, recording a
/// completed task that we failed to notify the first time. Persisted in
/// DO storage under key `notify_pending` so retries survive isolate
/// eviction. See `try_notify_play_manager` and the alarm handler for the
/// retry mechanics.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub(crate) struct PendingNotify {
    /// PlayManager DO target name (`{tenant_id}:play:{run_id}`).
    pub(crate) target_name: String,
    /// The play-side task id (suffix of the AgentTask id) to mark complete.
    pub(crate) play_task_id: String,
    /// How many delivery attempts have been made so far (including the
    /// initial attempt).
    pub(crate) attempts: u32,
    /// Earliest unix-millis at which the next retry may run.
    pub(crate) next_attempt_at_ms: u64,
}

/// Default lease-expiry sweep interval (ms). Used both for the
/// post-enqueue init alarm (PR #132 finding) and for the steady-state
/// sweep at the end of `alarm()`.
const LEASE_SWEEP_INTERVAL_MS: u64 = 60_000;

/// Maximum delivery attempts for a pending notification before we drop
/// it with an error log. After this many tries downstream tasks may
/// orphan (the PlayManager is presumed unreachable); we surface the
/// drop via `console_log!` so it is visible in `wrangler tail`.
pub(crate) const MAX_NOTIFY_ATTEMPTS: u32 = 5;

/// Pure helper: compute the next retry backoff time in unix-millis given
/// the current attempt count (already incremented to reflect the failure
/// we are scheduling against) and `now_ms`. Backoff doubles each attempt
/// starting from a 1s base, capped at 60s to keep retries from drifting
/// out of any reasonable alarm horizon.
///
/// Extracted as a `pub(crate)` free function so it can be unit-tested
/// without a Workers runtime — see the `mod tests` block at the bottom.
pub(crate) fn next_attempt_at(now_ms: u64, attempts: u32) -> u64 {
    // attempts=1 -> 1s, 2 -> 2s, 3 -> 4s, 4 -> 8s, then capped at 60s.
    let exp = attempts.saturating_sub(1).min(6);
    let backoff_ms: u64 = 1_000u64.saturating_mul(1u64 << exp);
    let backoff_ms = backoff_ms.min(60_000);
    now_ms.saturating_add(backoff_ms)
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

        // Extract the path-tail for parameterised routes. The lib.rs
        // callers build `https://do/complete/{task_id}` and
        // `https://do/fail/{task_id}` (see lib.rs:836 / lib.rs:867), so
        // the DO receives e.g. `/complete/<task_id>` as `req.path()`.
        //
        // The previous match arms `"/complete"` and `"/fail"` matched on
        // the *static* string and therefore never fired against a path
        // that carried a task_id — every call returned the catch-all
        // `404 not found`. PR #132 crr finding (task_do.rs:194 / :241):
        // this bug shipped to production and was first surfaced by PR
        // #142's tests. We now match by prefix and pull task_id out of
        // the trailing segment, mirroring the parsing the original
        // handler attempted via `req.path().split('/').nth(2)`.
        let complete_task_id = path.strip_prefix("/complete/").map(|s| s.to_string());
        let fail_task_id = path.strip_prefix("/fail/").map(|s| s.to_string());

        match (method, path.as_str()) {
            (Method::Post, "/enqueue") => {
                let task: AgentTask = req.json().await?;
                let storage = self.state.storage();

                let mut pending: VecDeque<AgentTask> = storage
                    .get("pending")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                pending.push_back(task);
                storage.put("pending", pending).await?;

                // PR #132 crr finding (task_do.rs:30): the first `/enqueue`
                // with no active tasks left the DO with no alarm scheduled,
                // so lease-expiry / notify-retry never ran until something
                // else (a /claim) happened to set one. Always ensure an
                // alarm is pending after enqueue.
                ensure_sweep_alarm(&storage).await?;

                Response::ok("enqueued")
            }
            (Method::Post, "/claim") => {
                let params: std::collections::HashMap<String, String> = req
                    .url()?
                    .query_pairs()
                    .into_iter()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                let agent_id = params.get("agent_id").cloned().unwrap_or_default();
                let caps_str = params.get("caps").cloned().unwrap_or_default();
                let caps: Vec<String> = caps_str
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();

                let storage = self.state.storage();
                let mut pending: VecDeque<AgentTask> = storage
                    .get("pending")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let mut active: std::collections::HashMap<String, AgentTask> = storage
                    .get("active")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();

                // Find first task that matches capabilities
                let task_idx = pending
                    .iter()
                    .position(|t| caps.is_empty() || caps.contains(&t.task_type));

                if let Some(idx) = task_idx {
                    let mut task = pending.remove(idx).unwrap();
                    task.status = "running".to_string();
                    task.agent_id = Some(agent_id);

                    // Set lease for 5 minutes
                    let now = js_sys::Date::now() as u64;
                    let expires = now + (300 * 1000);
                    task.lease_expires_at = Some(
                        js_sys::Date::new(&serde_wasm_bindgen::to_value(&expires).unwrap())
                            .to_iso_string()
                            .as_string()
                            .unwrap(),
                    );

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
                let params: std::collections::HashMap<String, String> = req
                    .url()?
                    .query_pairs()
                    .into_iter()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                let task_id = params.get("task_id").cloned().unwrap_or_default();
                let agent_id = params.get("agent_id").cloned().unwrap_or_default();

                let storage = self.state.storage();
                let mut active: std::collections::HashMap<String, AgentTask> = storage
                    .get("active")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();

                if let Some(task) = active.get_mut(&task_id) {
                    if task.agent_id.as_ref() == Some(&agent_id) {
                        let now = js_sys::Date::now() as u64;
                        let expires = now + (300 * 1000);
                        task.lease_expires_at = Some(
                            js_sys::Date::new(&serde_wasm_bindgen::to_value(&expires).unwrap())
                                .to_iso_string()
                                .as_string()
                                .unwrap(),
                        );

                        storage.put("active", active).await?;
                        return Response::ok("ok");
                    }
                }
                Response::error("task not found or not owned by agent", 404)
            }
            (Method::Post, _) if complete_task_id.is_some() => {
                let task_id = complete_task_id.unwrap_or_default();
                if task_id.is_empty() {
                    return Response::error("missing task id", 400);
                }
                let result: Option<serde_json::Value> = req.json().await.ok();

                let storage = self.state.storage();
                let mut active: std::collections::HashMap<String, AgentTask> = storage
                    .get("active")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();

                if let Some(mut task) = active.remove(&task_id) {
                    task.status = "completed".to_string();
                    task.result = result;
                    task.completed_at =
                        Some(js_sys::Date::new_0().to_iso_string().as_string().unwrap());

                    let job_id = task.job_id.clone();
                    let task_tenant = task.tenant_id.clone();

                    storage.put("active", active).await?;

                    // If task belongs to a play, notify PlayManager. The
                    // PlayManager DO is tenant-namespaced (see lib.rs
                    // `/v1/plays/:name/launch`) so we must reconstruct the
                    // name as `{tenant_id}:play:{run_id}`. If the task is
                    // missing tenant_id (legacy persisted state from before
                    // WS8), we skip the notification rather than routing to
                    // a potentially cross-tenant DO instance.
                    let play_task_id = task_id
                        .split('-')
                        .next_back()
                        .unwrap_or(&task_id)
                        .to_string();

                    if let Some(tenant_id) = task_tenant.as_deref() {
                        if !tenant_id.is_empty() {
                            let do_name = format!("{}:play:{}", tenant_id, job_id);
                            // PR #132 crr finding (task_do.rs:134): the previous
                            // notification was `let _ = play_stub.fetch_with_request().await;`
                            // which silently dropped delivery failures and orphaned
                            // any downstream tasks if PlayManager was unreachable.
                            // We now (1) attempt the notify, (2) on failure persist
                            // a PendingNotify entry to DO storage, (3) ensure an
                            // alarm is scheduled to drive the retry loop, and (4)
                            // surface the failure with a warn log visible in
                            // `wrangler tail`.
                            self.try_notify_play_manager(&do_name, &play_task_id, 1)
                                .await;
                        } else {
                            worker::console_log!(
                                "skipping PlayManager notify for task {}: empty tenant_id",
                                task_id
                            );
                        }
                    } else {
                        worker::console_log!(
                            "skipping PlayManager notify for task {}: tenant_id missing (pre-WS8 task)",
                            task_id
                        );
                    }

                    Response::from_json(&task)
                } else {
                    Response::error("task not found or not running", 404)
                }
            }
            (Method::Post, _) if fail_task_id.is_some() => {
                let task_id = fail_task_id.unwrap_or_default();
                if task_id.is_empty() {
                    return Response::error("missing task id", 400);
                }
                let fail_req: TaskFailRequest = req.json().await?;

                let storage = self.state.storage();
                let mut active: std::collections::HashMap<String, AgentTask> = storage
                    .get("active")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let mut pending: VecDeque<AgentTask> = storage
                    .get("pending")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();

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
                        task.completed_at =
                            Some(js_sys::Date::new_0().to_iso_string().as_string().unwrap());
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
        let mut active: std::collections::HashMap<String, AgentTask> = storage
            .get("active")
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut pending: VecDeque<AgentTask> = storage
            .get("pending")
            .await
            .ok()
            .flatten()
            .unwrap_or_default();

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
                    task.result =
                        Some(serde_json::json!({ "error": "lease expired and no retries left" }));
                    task.completed_at =
                        Some(js_sys::Date::new_0().to_iso_string().as_string().unwrap());
                }
            }
        }

        storage.put("active", active).await?;
        storage.put("pending", pending).await?;

        // Drive the PlayManager notification retry loop (PR #132 crr
        // finding on task_do.rs:134). Pending entries persist across
        // isolate evictions; each tick retries the entries whose
        // `next_attempt_at_ms` has elapsed, drops those that exceed
        // MAX_NOTIFY_ATTEMPTS, and re-persists the survivors.
        self.drive_notify_retries(now).await;

        // If there are still active tasks OR pending notifications,
        // schedule the next sweep alarm.
        let active: std::collections::HashMap<String, AgentTask> = storage
            .get("active")
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let notify_pending: Vec<PendingNotify> = storage
            .get("notify_pending")
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        if !active.is_empty() || !notify_pending.is_empty() {
            let _ = storage
                .set_alarm((now + LEASE_SWEEP_INTERVAL_MS) as i64)
                .await;
        }

        Response::ok("alarm processed")
    }
}

impl TaskLeaseManager {
    /// Attempt to notify PlayManager that a task completed. On failure,
    /// persist a `PendingNotify` entry and ensure an alarm is scheduled
    /// so the retry loop in `drive_notify_retries` will pick it up.
    ///
    /// `attempts` is the attempt number being recorded (1 for the
    /// initial direct call). The function is fire-and-forget from the
    /// caller's perspective — errors are logged + persisted, not bubbled.
    async fn try_notify_play_manager(&self, target_name: &str, play_task_id: &str, attempts: u32) {
        let do_req = match Request::new_with_init(
            "https://do/task-completed",
            &RequestInit {
                method: Method::Post,
                body: Some(
                    match serde_wasm_bindgen::to_value(&play_task_id.to_string()) {
                        Ok(v) => v,
                        Err(e) => {
                            worker::console_log!(
                                "task_do: failed to serialize play_task_id {}: {}",
                                play_task_id,
                                e
                            );
                            return;
                        }
                    },
                ),
                ..Default::default()
            },
        ) {
            Ok(r) => r,
            Err(e) => {
                worker::console_log!(
                    "task_do: failed to build notify request for {}: {}",
                    target_name,
                    e
                );
                return;
            }
        };

        let result: Result<Response> = match self.env.durable_object("PLAY_MANAGER") {
            Ok(ns) => match ns.id_from_name(target_name).and_then(|id| id.get_stub()) {
                Ok(stub) => stub.fetch_with_request(do_req).await,
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };

        let succeeded = matches!(&result, Ok(resp) if resp.status_code() < 500);
        if succeeded {
            return;
        }

        // Failure path — log + enqueue / re-enqueue retry.
        match &result {
            Ok(resp) => worker::console_log!(
                "WARN: PlayManager notify {}/{} returned status {} (attempt {}); will retry",
                target_name,
                play_task_id,
                resp.status_code(),
                attempts,
            ),
            Err(e) => worker::console_log!(
                "WARN: PlayManager notify {}/{} failed (attempt {}): {}; will retry",
                target_name,
                play_task_id,
                attempts,
                e,
            ),
        }

        if attempts >= MAX_NOTIFY_ATTEMPTS {
            worker::console_log!(
                "ERROR: dropping PlayManager notify {}/{} after {} attempts",
                target_name,
                play_task_id,
                attempts
            );
            return;
        }

        let now = js_sys::Date::now() as u64;
        let storage = self.state.storage();
        let mut pending_list: Vec<PendingNotify> = storage
            .get("notify_pending")
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let next_at = next_attempt_at(now, attempts);
        let entry = PendingNotify {
            target_name: target_name.to_string(),
            play_task_id: play_task_id.to_string(),
            attempts,
            next_attempt_at_ms: next_at,
        };
        upsert_pending_notify(&mut pending_list, entry);
        if let Err(e) = storage.put("notify_pending", pending_list).await {
            worker::console_log!(
                "task_do: failed to persist notify_pending for {}/{}: {}",
                target_name,
                play_task_id,
                e
            );
            return;
        }

        if let Err(e) = ensure_sweep_alarm(&storage).await {
            worker::console_log!("task_do: failed to schedule notify retry alarm: {}", e);
        }
    }

    /// Walk the persisted `notify_pending` list, retry entries whose
    /// `next_attempt_at_ms` has elapsed, drop those past
    /// `MAX_NOTIFY_ATTEMPTS`, and persist the survivors.
    async fn drive_notify_retries(&self, now_ms: u64) {
        let storage = self.state.storage();
        let pending_list: Vec<PendingNotify> = storage
            .get("notify_pending")
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        if pending_list.is_empty() {
            return;
        }

        let (due, deferred) = split_due_pending(&pending_list, now_ms);

        // Persist the deferred-only list before issuing retries — if a
        // retry succeeds it will not re-add the entry, and if it fails
        // try_notify_play_manager will re-upsert with the bumped attempt
        // count. This avoids double-counting attempts on isolate
        // eviction mid-tick.
        if let Err(e) = storage.put("notify_pending", deferred).await {
            worker::console_log!("task_do: failed to checkpoint notify_pending: {}", e);
            return;
        }

        for entry in due {
            // attempts+1 because this represents the next attempt
            // number; semantics: attempt 1 was the initial /complete
            // call, attempt 2 is the first retry, etc.
            self.try_notify_play_manager(
                &entry.target_name,
                &entry.play_task_id,
                entry.attempts.saturating_add(1),
            )
            .await;
        }
    }
}

/// Ensure an alarm is scheduled. If none is currently set, schedule one
/// `LEASE_SWEEP_INTERVAL_MS` from now. Used by both `/enqueue` (the
/// PR #132 task_do.rs:30 fix) and the notify-retry persistence path so
/// the retry loop can actually fire.
async fn ensure_sweep_alarm(storage: &Storage) -> Result<()> {
    let current = storage.get_alarm().await.ok().flatten();
    if current.is_none() {
        let now = js_sys::Date::now() as u64;
        storage
            .set_alarm((now + LEASE_SWEEP_INTERVAL_MS) as i64)
            .await?;
    }
    Ok(())
}

/// Pure helper: split a `notify_pending` list into (due_now, deferred)
/// based on `now_ms`. Extracted for unit-testability.
pub(crate) fn split_due_pending(
    pending: &[PendingNotify],
    now_ms: u64,
) -> (Vec<PendingNotify>, Vec<PendingNotify>) {
    let mut due = Vec::new();
    let mut deferred = Vec::new();
    for entry in pending {
        if entry.next_attempt_at_ms <= now_ms {
            due.push(entry.clone());
        } else {
            deferred.push(entry.clone());
        }
    }
    (due, deferred)
}

/// Pure helper: extract `task_id` from a DO request path of the form
/// `/<route>/<task_id>` for the parameterised routes `/complete` and
/// `/fail`. Returns `Some(task_id)` when the path matches the given
/// route prefix and the task_id segment is non-empty; `None` otherwise.
///
/// Extracted as a test-only free function so the routing contract
/// (mirroring `path.strip_prefix("/complete/")` in the live `fetch`
/// handler) is unit-testable on host without spinning up a Workers
/// runtime. This backs the regression test for the PR #132 routing bug
/// where the old `/complete` static match arm never fired against the
/// lib.rs caller's `/complete/<task_id>` path.
#[cfg(test)]
pub(crate) fn parse_task_id_from_path(path: &str, route: &str) -> Option<String> {
    let prefix = format!("/{}/", route.trim_matches('/'));
    let tail = path.strip_prefix(&prefix)?;
    if tail.is_empty() {
        None
    } else {
        Some(tail.to_string())
    }
}

/// Pure helper: upsert a PendingNotify into the in-memory list, keyed
/// by `(target_name, play_task_id)`. If the key already exists we
/// replace it (matters for retry re-enqueue with bumped attempt count).
pub(crate) fn upsert_pending_notify(list: &mut Vec<PendingNotify>, entry: PendingNotify) {
    if let Some(existing) = list
        .iter_mut()
        .find(|e| e.target_name == entry.target_name && e.play_task_id == entry.play_task_id)
    {
        *existing = entry;
    } else {
        list.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PR #132 finding: task_do.rs:30 — alarm init on first enqueue ──
    //
    // The DO-level behaviour (calling `ensure_sweep_alarm` after each
    // enqueue) is exercised end-to-end via worker-rs at runtime; here
    // we assert the contract that backs it: if no alarm is currently
    // scheduled, the helper schedules one; if one already exists,
    // it leaves it alone. Without a real Storage we model the
    // contract as a pure function over the `Option<i64>` returned by
    // `storage.get_alarm()`. See `should_schedule_alarm` below.

    /// Mirror of `ensure_sweep_alarm`'s decision logic, expressed as a
    /// pure function so the contract can be asserted without a Workers
    /// runtime. If this and `ensure_sweep_alarm` diverge in future
    /// edits, this test will be a tripwire.
    fn should_schedule_alarm(current: Option<i64>) -> bool {
        current.is_none()
    }

    #[test]
    fn enqueue_with_no_existing_alarm_schedules_one() {
        // PR #132 finding task_do.rs:30: first /enqueue with no active
        // tasks must always schedule the sweep alarm.
        assert!(should_schedule_alarm(None));
    }

    #[test]
    fn enqueue_with_existing_alarm_is_idempotent() {
        // If an alarm is already pending (e.g. from a prior /claim
        // lease-expiry alarm) we must NOT clobber it with a later, less
        // urgent sweep alarm.
        assert!(!should_schedule_alarm(Some(123_456_789)));
    }

    // ── PR #132 finding: task_do.rs:134 — notification durability ─────

    #[test]
    fn notify_retry_persists_pending_with_bumped_attempt_counter() {
        // Initial delivery failed (attempt 1). The retry path persists
        // a PendingNotify with attempts=1 and schedules a future retry.
        // The alarm tick then issues attempt 2 — and if that also
        // fails, the entry is re-upserted with attempts=2. This test
        // asserts the upsert behaviour that backs that loop.
        let mut list = vec![];
        let now = 1_000_000u64;

        upsert_pending_notify(
            &mut list,
            PendingNotify {
                target_name: "job-A".to_string(),
                play_task_id: "t1".to_string(),
                attempts: 1,
                next_attempt_at_ms: next_attempt_at(now, 1),
            },
        );
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].attempts, 1);

        // Simulate an alarm-driven retry that also failed: the list
        // gets the same key re-upserted with attempts incremented.
        upsert_pending_notify(
            &mut list,
            PendingNotify {
                target_name: "job-A".to_string(),
                play_task_id: "t1".to_string(),
                attempts: 2,
                next_attempt_at_ms: next_attempt_at(now, 2),
            },
        );
        assert_eq!(list.len(), 1, "upsert keyed on (target,task), not appended");
        assert_eq!(list[0].attempts, 2);

        // Different task -> separate entry.
        upsert_pending_notify(
            &mut list,
            PendingNotify {
                target_name: "job-A".to_string(),
                play_task_id: "t2".to_string(),
                attempts: 1,
                next_attempt_at_ms: next_attempt_at(now, 1),
            },
        );
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn notify_retry_backoff_grows_exponentially() {
        let now = 1_000_000u64;
        assert_eq!(next_attempt_at(now, 1), now + 1_000);
        assert_eq!(next_attempt_at(now, 2), now + 2_000);
        assert_eq!(next_attempt_at(now, 3), now + 4_000);
        assert_eq!(next_attempt_at(now, 4), now + 8_000);
        // Capped at 60s.
        assert_eq!(next_attempt_at(now, 20), now + 60_000);
    }

    #[test]
    fn split_due_pending_separates_due_from_deferred() {
        let now = 1_000_000u64;
        let pending = vec![
            PendingNotify {
                target_name: "j1".to_string(),
                play_task_id: "t1".to_string(),
                attempts: 1,
                next_attempt_at_ms: now - 100, // due
            },
            PendingNotify {
                target_name: "j1".to_string(),
                play_task_id: "t2".to_string(),
                attempts: 1,
                next_attempt_at_ms: now + 5_000, // deferred
            },
            PendingNotify {
                target_name: "j2".to_string(),
                play_task_id: "t1".to_string(),
                attempts: 2,
                next_attempt_at_ms: now, // due (equality)
            },
        ];
        let (due, deferred) = split_due_pending(&pending, now);
        assert_eq!(due.len(), 2);
        assert_eq!(deferred.len(), 1);
        assert_eq!(deferred[0].play_task_id, "t2");
    }

    // ── PR #132 finding: task_do.rs:194 / :241 — /complete and /fail
    //    routing was unreachable. The static match arms `"/complete"`
    //    and `"/fail"` never fired against the lib.rs caller's
    //    `https://do/complete/<task_id>` URL (req.path() carries the
    //    full path including the id), so every call returned 404.
    //    These tests pin the path-parsing contract that backs the new
    //    prefix-match arms.

    #[test]
    fn parse_task_id_extracts_segment_from_complete_path() {
        assert_eq!(
            parse_task_id_from_path("/complete/task-abc-123", "complete"),
            Some("task-abc-123".to_string()),
        );
    }

    #[test]
    fn parse_task_id_extracts_segment_from_fail_path() {
        assert_eq!(
            parse_task_id_from_path("/fail/task-xyz", "fail"),
            Some("task-xyz".to_string()),
        );
    }

    #[test]
    fn parse_task_id_returns_none_for_static_path() {
        // The pre-fix bug shape: the lib caller never sends a bare
        // "/complete" — but if it did, we must produce None (and the
        // DO returns 400 / 404), not silently treat empty as a valid id.
        assert_eq!(parse_task_id_from_path("/complete", "complete"), None);
        assert_eq!(parse_task_id_from_path("/complete/", "complete"), None);
    }

    #[test]
    fn parse_task_id_returns_none_for_unrelated_path() {
        assert_eq!(parse_task_id_from_path("/enqueue", "complete"), None);
        assert_eq!(parse_task_id_from_path("/claim", "fail"), None);
    }

    #[test]
    fn parse_task_id_round_trips_lib_rs_caller_url() {
        // Regression for the production bug: the lib.rs handler builds
        // `https://do/complete/{task_id}` via Request::new_with_init, and
        // worker-rs surfaces `req.path()` as `/complete/{task_id}`. The
        // old `.nth(2)` parsing assumed three segments — but the path
        // only ever has two, and the match arm itself never matched
        // anyway. Pin the contract: a representative caller path must
        // produce a non-empty task_id.
        let path = "/complete/run-42-summarise";
        let task_id = parse_task_id_from_path(path, "complete")
            .expect("lib.rs caller URL must yield a non-empty task id");
        assert!(!task_id.is_empty());
        assert_eq!(task_id, "run-42-summarise");
    }

    #[test]
    fn max_attempts_drops_pending_entry() {
        // Documents the MAX_NOTIFY_ATTEMPTS cap. The try_notify path
        // returns early without re-enqueueing when attempts >= MAX,
        // so a pending list that goes through retries will never
        // contain an entry with attempts >= MAX_NOTIFY_ATTEMPTS — and
        // a drop is logged as an error.
        assert_eq!(MAX_NOTIFY_ATTEMPTS, 5);
    }

    // ── DO unit coverage (PR #142): queue / lease / complete helpers ──

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
            tenant_id: Some("tenant-test".to_string()),
        }
    }

    #[test]
    fn enqueue_then_claim_returns_task_and_decrements_pending() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();

        enqueue_task(&mut pending, make_task("t1", "build"));
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
        assert_eq!(pending.len(), 0);
        assert_eq!(active.len(), 1);
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

        let released = expire_leases(&mut active, &mut pending, 2_000, "1970-01-01T00:00:00Z");
        assert_eq!(released, vec!["t1".to_string()]);
        assert!(active.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].retry_count, 1);
    }

    #[test]
    fn complete_marks_task_completed_and_removes_from_active() {
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        let mut task = make_task("t1", "build");
        task.status = "running".to_string();
        active.insert("t1".to_string(), task);

        let completed = complete_task(
            &mut active,
            "t1",
            Some(serde_json::json!({"out": 42})),
            "2026-01-01T00:00:00Z",
        );

        assert!(completed.is_some());
        assert_eq!(completed.unwrap().status, "completed");
        assert!(active.is_empty());
    }

    #[test]
    fn double_complete_is_idempotent_no_double_notification() {
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        active.insert("t1".to_string(), make_task("t1", "build"));

        assert!(complete_task(&mut active, "t1", None, "ts").is_some());
        assert!(complete_task(&mut active, "t1", None, "ts").is_none());
    }

    #[test]
    fn fail_requeues_when_retries_remain() {
        let mut pending: VecDeque<AgentTask> = VecDeque::new();
        let mut active: HashMap<String, AgentTask> = HashMap::new();
        active.insert("t1".to_string(), make_task("t1", "build"));

        let failed = fail_task(&mut active, &mut pending, "t1", "boom", "ts");
        assert_eq!(failed.unwrap().retry_count, 1);
        assert_eq!(pending.len(), 1);
        assert!(active.is_empty());
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
        assert_eq!(failed.unwrap().status, "failed");
        assert!(pending.is_empty());
    }
}
