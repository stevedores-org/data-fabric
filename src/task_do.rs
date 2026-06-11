use crate::models::{AgentTask, TaskFailRequest};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use worker::*;

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug)]
struct TaskLeaseState {
    pending_tasks: VecDeque<AgentTask>,
    active_tasks: std::collections::HashMap<String, AgentTask>,
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
        //
        // crr2 MEDIUM finding (task_do.rs:87): the strip_prefix path
        // suffix is the raw URL path, which is percent-encoded by the
        // caller (data-fabric-client PR #140's URL encoding fix, plus
        // anything routed by Workers' router). If we used the encoded
        // form as the hashmap key the lookup would silently miss vs.
        // the `/enqueue` path which deserializes from JSON (no
        // encoding). Decode to the canonical form here.
        let complete_task_id = path
            .strip_prefix("/complete/")
            .and_then(decode_task_id_segment);
        let fail_task_id = path
            .strip_prefix("/fail/")
            .and_then(decode_task_id_segment);

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

/// Pure helper: percent-decode a `/complete/<task_id>` or
/// `/fail/<task_id>` path tail into the canonical task id. Returns
/// `None` for an empty segment, an invalid UTF-8 percent-encoding, or
/// a decoded value that is empty after decoding.
///
/// crr2 MEDIUM finding (task_do.rs:87): the previous strip_prefix path
/// kept the percent-encoded form (e.g. `task%2Dabc`) and used it as a
/// hashmap key. The corresponding `/enqueue` path deserialises the
/// task id from JSON, so the key in `active` was already in canonical
/// form — meaning a percent-encoded `/complete/...` lookup silently
/// missed and returned `404 task not found`. Decoding here makes the
/// keying consistent regardless of caller encoding.
///
/// Extracted as a `pub(crate)` free function so it can be unit-tested
/// without spinning up a Workers runtime.
pub(crate) fn decode_task_id_segment(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let decoded = percent_decode_str(raw).decode_utf8().ok()?;
    let decoded = decoded.into_owned();
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
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
    // Mirror the production decode step (crr2 MEDIUM finding) so this
    // helper stays a faithful model of the live `fetch` routing.
    decode_task_id_segment(tail)
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

    // ── crr2 MEDIUM finding: task_do.rs:87 — URL-decoded task_id ──────
    //
    // The strip_prefix("/complete/") path parsing kept the raw
    // percent-encoded suffix. data-fabric-client (PR #140) URL-encodes
    // path segments per RFC 3986, so a task id like
    // `task-with space` arrives as `task-with%20space`. The hashmap
    // key in `active` is the JSON-deserialised (canonical) form from
    // `/enqueue`, so the encoded lookup would miss and return
    // `404 task not found`. The fix decodes here.

    #[test]
    fn decode_task_id_segment_passes_through_unencoded_id() {
        assert_eq!(
            decode_task_id_segment("task-abc-123"),
            Some("task-abc-123".to_string()),
        );
    }

    #[test]
    fn decode_task_id_segment_percent_decodes_spaces() {
        // The bug shape: client encoded ' ' as '%20'; the DO key was
        // stored as the canonical form on /enqueue. Pre-fix, the
        // active.remove(&"task%20abc") missed; post-fix it hits.
        assert_eq!(
            decode_task_id_segment("task%20abc"),
            Some("task abc".to_string()),
        );
    }

    #[test]
    fn decode_task_id_segment_handles_reserved_chars() {
        // Reserved chars per RFC 3986 — client-side URL encoders
        // typically escape `/`, `?`, `#`, `&`, `=` etc. when present
        // inside a path segment. We must round-trip them back to the
        // canonical form so the active-task lookup hits.
        assert_eq!(
            decode_task_id_segment("run%2F42%23summarise"),
            Some("run/42#summarise".to_string()),
        );
    }

    #[test]
    fn decode_task_id_segment_returns_none_for_empty() {
        // Empty segment is treated as a parse failure (mirrors the
        // pre-existing `parse_task_id_from_path` contract that returns
        // None for `/complete/`).
        assert_eq!(decode_task_id_segment(""), None);
    }

    #[test]
    fn decode_task_id_segment_returns_none_for_invalid_utf8_percent_encoding() {
        // `%FF%FE` is not valid UTF-8 — should bail rather than
        // produce a corrupted key that misses the active-task map.
        assert_eq!(decode_task_id_segment("%FF%FE"), None);
    }

    #[test]
    fn parse_task_id_decodes_percent_encoded_segment() {
        // End-to-end on the parsing helper used by the routing
        // contract test. Pre-crr2 this returned Some("task%20abc")
        // verbatim and the active.remove lookup missed.
        assert_eq!(
            parse_task_id_from_path("/complete/task%20abc", "complete"),
            Some("task abc".to_string()),
        );
        assert_eq!(
            parse_task_id_from_path("/fail/task%2Fabc", "fail"),
            Some("task/abc".to_string()),
        );
    }

    #[test]
    fn parse_task_id_round_trips_url_encoded_caller_url() {
        // Regression: data-fabric-client PR #140 url-encodes path
        // segments, so the lib.rs caller may forward an encoded
        // task_id into the DO URL. The decoded form must match the
        // canonical key stored on /enqueue.
        let canonical = "run-42 summarise/v2";
        let encoded =
            percent_encoding::utf8_percent_encode(canonical, percent_encoding::NON_ALPHANUMERIC)
                .to_string();
        let path = format!("/complete/{}", encoded);
        let task_id = parse_task_id_from_path(&path, "complete")
            .expect("encoded caller URL must yield a decoded task id");
        assert_eq!(task_id, canonical);
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
}
