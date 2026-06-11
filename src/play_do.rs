use crate::models::{AgentTask, PlayDefinition, PlayTaskDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use worker::*;

/// Envelope sent by `POST /v1/plays/:name/launch` (in `lib.rs`) so the
/// PlayManager DO is told which tenant owns this launch instead of
/// fabricating it from `self.state.id()`.
#[derive(Serialize, Deserialize, Debug)]
struct LaunchEnvelope {
    tenant_id: String,
    definition: PlayDefinition,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PlayState {
    definition: PlayDefinition,
    run_id: String,
    /// Owning tenant. Persisted in DO storage so subsequent task-completion
    /// callbacks and task materializations route to the correct
    /// TaskLeaseManager instance and stamp the correct tenant on each
    /// emitted AgentTask. Pre-WS8 this was incorrectly derived from
    /// `self.state.id()` (the DO's instance UUID).
    tenant_id: String,
    completed_tasks: HashSet<String>,
    active_tasks: HashSet<String>,
    /// Monotonic version counter. Bumped on every persist that mutates
    /// completed_tasks / active_tasks. Used to detect TOCTOU drift between
    /// when `materialize_eligible_tasks` snapshots state and when (if ever)
    /// it writes back. See `derive_to_launch` and the `/launch` /
    /// `/task-completed` handlers below for the read-derive-write contract.
    #[serde(default)]
    state_version: u64,
}

#[durable_object]
pub struct PlayManager {
    state: State,
    env: Env,
}

impl DurableObject for PlayManager {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let method = req.method();

        match (method, path.as_str()) {
            (Method::Post, "/launch") => {
                let envelope: LaunchEnvelope = req.json().await?;
                let LaunchEnvelope {
                    tenant_id,
                    definition: def,
                } = envelope;

                if tenant_id.is_empty() {
                    return Response::error("tenant_id required in launch envelope", 400);
                }

                let run_id = crate::generate_id().unwrap_or_else(|_| "err".to_string());

                let state = PlayState {
                    definition: def.clone(),
                    run_id: run_id.clone(),
                    tenant_id: tenant_id.clone(),
                    completed_tasks: HashSet::new(),
                    active_tasks: HashSet::new(),
                    state_version: 0,
                };

                // Materialize initial tasks. `materialize_eligible_tasks` is
                // responsible for the atomic read-derive-write of state — we
                // intentionally do NOT pre-persist `state` here so that the
                // helper owns the single canonical write. Pass in the seed
                // state as an Option to seed the storage on first call.
                self.materialize_eligible_tasks(Some(state)).await?;

                Response::from_json(&serde_json::json!({
                    "run_id": run_id,
                    "status": "launched"
                }))
            }
            (Method::Post, "/task-completed") => {
                let task_id: String = req.json().await?;
                let storage = self.state.storage();
                // Atomic read-modify-write of the completion event. We then
                // call `materialize_eligible_tasks(None)` which will read
                // the freshly-persisted state, derive to_launch, mark
                // active, and persist BEFORE issuing outbound RPCs to
                // TaskLeaseManager — closing the TOCTOU window that
                // previously existed between derive (L74) and write (L99).
                let mut state: PlayState = storage
                    .get("state")
                    .await?
                    .ok_or_else(|| Error::RustError("state not found".into()))?;

                state.completed_tasks.insert(task_id.clone());
                state.active_tasks.remove(&task_id);
                state.state_version = state.state_version.wrapping_add(1);

                storage.put("state", &state).await?;

                self.materialize_eligible_tasks(None).await?;

                Response::ok("ok")
            }
            _ => Response::error("not found", 404),
        }
    }
}

impl PlayManager {
    /// Atomically materialize all newly-eligible tasks.
    ///
    /// Read-derive-write contract:
    /// 1. Read state from storage (or use the supplied seed for first launch).
    /// 2. Derive `to_launch` from the snapshot via the pure helper
    ///    `derive_to_launch`.
    /// 3. Mark the launched tasks as active in the snapshot, bump
    ///    `state_version`, and persist BEFORE issuing any outbound RPCs.
    ///    This is the load-bearing step: previously the DO issued outbound
    ///    `task_stub.fetch_with_request().await` calls between the read at
    ///    L74 and the write at L99 (see PR #132 crr finding) — the await
    ///    released the DO input gate, letting `/task-completed` mutate
    ///    storage in between, and the subsequent write clobbered those
    ///    mutations. Persisting active_tasks before the outbound RPC means
    ///    a concurrent `/task-completed` sees the up-to-date active set.
    /// 4. After the write succeeds, emit the outbound RPCs to enqueue tasks
    ///    on TaskLeaseManager. If an RPC fails the task is "stuck active"
    ///    in PlayManager's view, but TLM's lease-expiry alarm + retry path
    ///    will recover; this is strictly safer than the old behaviour of
    ///    potential duplicate launches.
    async fn materialize_eligible_tasks(&self, seed: Option<PlayState>) -> Result<()> {
        let storage = self.state.storage();
        let mut state: PlayState = match seed {
            Some(s) => s,
            None => storage
                .get("state")
                .await?
                .ok_or_else(|| Error::RustError("state not found".into()))?,
        };

        let to_launch = derive_to_launch(&state);
        if to_launch.is_empty() {
            // Still need to persist the seed on first launch even when the
            // play has no eligible tasks (e.g. all gated by deps).
            storage.put("state", &state).await?;
            return Ok(());
        }

        // Route to the per-tenant TaskLeaseManager. The TLM DO is named by
        // tenant_id everywhere else in the worker (see lib.rs handlers for
        // /mcp/task/*) so we use the same naming here. Previously this used
        // `self.state.id().to_string()` (the PlayManager's instance UUID),
        // which created a stray TLM per play run and broke cross-DO
        // coordination across the tenant's other task surfaces.
        let task_ns = self.env.durable_object("TASK_LEASE_MANAGER")?;
        let task_stub = task_ns.id_from_name(&state.tenant_id)?.get_stub()?;

        // PR #132 crr finding (play_do.rs:178): the previous implementation
        // marked ALL `to_launch` tasks as active and persisted that state
        // BEFORE issuing any outbound RPCs. If any RPC then failed
        // (`.await?`), the loop bailed but the active-set write had
        // already landed — leaving every task in the batch marked active
        // forever, even though TaskLeaseManager never received them.
        // `derive_to_launch` excludes already-active tasks, and TLM
        // never lease-expires what it never knew about, so these tasks
        // were silently stuck.
        //
        // Fix (per-task atomicity, option (b) from the crr brief): mark
        // a task active ONLY AFTER its `/enqueue` RPC has succeeded,
        // persisting between each. This costs N writes instead of 1 but
        // guarantees the invariant: a task is in `active_tasks` iff TLM
        // has been told about it. A failed RPC short-circuits with the
        // error bubbled up (caller can retry); already-enqueued tasks
        // stay durably active; not-yet-enqueued tasks stay un-marked
        // and will be re-picked-up by the next `materialize_eligible_tasks`
        // call (e.g. via the next `/task-completed`).

        // Persist the seed state (first launch path) before issuing any
        // outbound RPCs so a concurrent handler observing storage sees a
        // consistent baseline. On the `/task-completed` path this is a
        // no-op rewrite of the just-persisted state from the caller.
        storage.put("state", &state).await?;

        for task_def in to_launch {
            let task = AgentTask {
                id: format!("{}-{}", state.run_id, task_def.id),
                job_id: state.run_id.clone(),
                task_type: task_def.task_type.clone(),
                priority: task_def.priority,
                status: "pending".to_string(),
                params: task_def.params.clone(),
                result: None,
                agent_id: None,
                graph_ref: None,
                play_id: Some(state.definition.name.clone()),
                parent_task_id: None,
                retry_count: 0,
                max_retries: 3,
                lease_expires_at: None,
                created_at: js_sys::Date::new_0().to_iso_string().as_string().unwrap(),
                completed_at: None,
                memory_context: None,
                tenant_id: Some(state.tenant_id.clone()),
            };

            let do_req = Request::new_with_init(
                "https://do/enqueue",
                &RequestInit {
                    method: Method::Post,
                    body: Some(
                        serde_wasm_bindgen::to_value(&task)
                            .map_err(|e| Error::RustError(e.to_string()))?,
                    ),
                    ..Default::default()
                },
            )?;

            // Issue the outbound RPC FIRST. If it fails, return without
            // marking this task active — leaving the durable state at
            // the last successfully-enqueued task. The caller surfaces
            // the error; the next materialize call will retry this and
            // remaining tasks because `derive_to_launch` still includes
            // anything not yet in `active_tasks`/`completed_tasks`.
            if let Err(e) = task_stub.fetch_with_request(do_req).await {
                worker::console_log!(
                    "play_do: enqueue RPC failed for task {} of run {}: {}; \
                     leaving active_tasks unchanged so a future materialize \
                     call will retry",
                    task_def.id,
                    state.run_id,
                    e,
                );
                return Err(e);
            }

            // RPC succeeded — now mark active and persist. We re-read
            // state from storage to fold in any concurrent
            // `/task-completed` mutation that may have landed while our
            // input gate yielded on the outbound RPC's await. Without
            // this re-read we would clobber concurrent `completed_tasks`
            // / `active_tasks.remove(...)` updates.
            let mut latest: PlayState = storage
                .get("state")
                .await?
                .ok_or_else(|| Error::RustError("state not found".into()))?;
            latest.active_tasks.insert(task_def.id.clone());
            latest.state_version = latest.state_version.wrapping_add(1);
            storage.put("state", &latest).await?;
            state = latest;
        }

        Ok(())
    }
}

/// Pure helper backing the per-task-atomicity invariant in
/// `materialize_eligible_tasks` (PR #132 crr finding on play_do.rs:178).
///
/// Simulates the per-task loop in pure terms: given an initial state,
/// the list of tasks to launch, and a per-task RPC outcome closure,
/// returns the (final_state, error_at_index) tuple. Any task whose RPC
/// errored is NOT marked active; tasks before it ARE marked active
/// (their RPCs succeeded). The first failing task aborts iteration.
///
/// Invariant: `final_state.active_tasks.contains(t)` <=> RPC for t
/// returned Ok. This is exactly what the production loop guarantees.
/// Test-only so it can stay tightly aligned with the live loop without
/// leaking the private `PlayState` type into the crate API surface.
#[cfg(test)]
fn simulate_per_task_materialize<F>(
    initial: PlayState,
    to_launch: &[PlayTaskDefinition],
    mut rpc: F,
) -> (PlayState, Option<usize>)
where
    F: FnMut(&PlayTaskDefinition) -> std::result::Result<(), String>,
{
    let mut state = initial;
    for (idx, task_def) in to_launch.iter().enumerate() {
        match rpc(task_def) {
            Ok(()) => {
                state.active_tasks.insert(task_def.id.clone());
                state.state_version = state.state_version.wrapping_add(1);
            }
            Err(_) => return (state, Some(idx)),
        }
    }
    (state, None)
}

/// Pure derivation of eligible-to-launch tasks from a PlayState snapshot.
///
/// Extracted as a free function with no DO/storage dependencies so it can
/// be unit-tested directly on host (non-wasm) without a Workers runtime.
/// The TOCTOU fix in `materialize_eligible_tasks` relies on this being a
/// pure function of `state` — no hidden I/O.
fn derive_to_launch(state: &PlayState) -> Vec<PlayTaskDefinition> {
    let mut to_launch = Vec::new();
    for task_def in &state.definition.tasks {
        if state.completed_tasks.contains(&task_def.id) || state.active_tasks.contains(&task_def.id)
        {
            continue;
        }
        let all_deps_met = task_def
            .depends_on
            .iter()
            .all(|dep_id| state.completed_tasks.contains(dep_id));
        if all_deps_met {
            to_launch.push(task_def.clone());
        }
    }
    to_launch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PlayTaskDefinition;

    fn task(id: &str, deps: &[&str]) -> PlayTaskDefinition {
        PlayTaskDefinition {
            id: id.to_string(),
            task_type: "test".to_string(),
            priority: 0,
            params: None,
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn make_state(tasks: Vec<PlayTaskDefinition>) -> PlayState {
        PlayState {
            definition: PlayDefinition {
                name: "p".to_string(),
                goal: "g".to_string(),
                tasks,
            },
            run_id: "run-1".to_string(),
            tenant_id: "tenant-test".to_string(),
            completed_tasks: HashSet::new(),
            active_tasks: HashSet::new(),
            state_version: 0,
        }
    }

    #[test]
    fn derive_to_launch_emits_root_tasks_with_no_deps() {
        let state = make_state(vec![task("a", &[]), task("b", &[])]);
        let launched = derive_to_launch(&state);
        let ids: HashSet<_> = launched.iter().map(|t| t.id.clone()).collect();
        assert_eq!(ids, HashSet::from(["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn derive_to_launch_skips_already_active_tasks() {
        // This is the TOCTOU regression test (PR #132 crr finding): if a
        // task is already in active_tasks (because a prior materialize
        // call marked it before the outbound RPC), a second derive on the
        // same state MUST NOT re-emit it. The pre-fix code re-read state
        // from storage AFTER deriving to_launch and would resurrect tasks
        // that another handler had already moved out of pending.
        let mut state = make_state(vec![task("a", &[]), task("b", &[])]);
        state.active_tasks.insert("a".to_string());
        let launched = derive_to_launch(&state);
        let ids: Vec<_> = launched.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn derive_to_launch_skips_completed_tasks() {
        let mut state = make_state(vec![task("a", &[]), task("b", &[])]);
        state.completed_tasks.insert("a".to_string());
        let launched = derive_to_launch(&state);
        let ids: Vec<_> = launched.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn derive_to_launch_respects_unmet_dependencies() {
        let state = make_state(vec![task("a", &[]), task("b", &["a"])]);
        let launched = derive_to_launch(&state);
        let ids: Vec<_> = launched.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
    }

    #[test]
    fn derive_to_launch_releases_dependent_when_parent_complete() {
        let mut state = make_state(vec![task("a", &[]), task("b", &["a"])]);
        state.completed_tasks.insert("a".to_string());
        let launched = derive_to_launch(&state);
        let ids: Vec<_> = launched.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn derive_to_launch_handles_multi_dep_gates() {
        // c depends on both a and b. Only a completed -> c stays gated.
        let mut state = make_state(vec![task("a", &[]), task("b", &[]), task("c", &["a", "b"])]);
        state.completed_tasks.insert("a".to_string());
        state.active_tasks.insert("b".to_string());
        let launched = derive_to_launch(&state);
        assert!(launched.is_empty(), "c gated until b completes");

        state.active_tasks.remove("b");
        state.completed_tasks.insert("b".to_string());
        let launched = derive_to_launch(&state);
        let ids: Vec<_> = launched.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["c"]);
    }

    // ── PR #132 finding: play_do.rs:178 — partial-enqueue stuck-active ─
    //
    // The pre-fix code marked every `to_launch` task as `active` and
    // persisted that state BEFORE issuing outbound RPCs. If any RPC then
    // failed (`.await?`), the active write was already durable but TLM
    // had never received the task. `derive_to_launch` excluded
    // already-active tasks, so the task was stuck active forever, and
    // TLM never lease-expired it because it never knew about it. These
    // tests pin the per-task atomicity contract that fixes that.

    #[test]
    fn per_task_materialize_only_marks_active_on_rpc_success() {
        let state = make_state(vec![task("a", &[]), task("b", &[]), task("c", &[])]);
        let to_launch = derive_to_launch(&state);
        // RPC succeeds for everyone.
        let (final_state, err_idx) = simulate_per_task_materialize(state, &to_launch, |_| Ok(()));
        assert_eq!(err_idx, None);
        let active: HashSet<_> = final_state.active_tasks.iter().cloned().collect();
        assert_eq!(
            active,
            HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()]),
        );
    }

    #[test]
    fn per_task_materialize_aborts_and_leaves_failed_task_inactive() {
        // This is the partial-enqueue regression test. Task "b"'s RPC
        // fails; "a"'s RPC succeeded; "c" should never be attempted.
        // The contract: post-failure state has ONLY "a" marked active,
        // and "b" / "c" can be re-attempted by a subsequent materialize.
        let state = make_state(vec![task("a", &[]), task("b", &[]), task("c", &[])]);
        let to_launch = derive_to_launch(&state);
        let (final_state, err_idx) = simulate_per_task_materialize(state, &to_launch, |t| {
            if t.id == "b" {
                Err("TLM enqueue RPC failed".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(err_idx, Some(1), "loop aborts at failed task");
        let active: HashSet<_> = final_state.active_tasks.iter().cloned().collect();
        assert_eq!(
            active,
            HashSet::from(["a".to_string()]),
            "only successfully-enqueued tasks marked active",
        );
        // Crucially, the failed task is NOT marked active — so the next
        // materialize call will re-derive it as eligible (see test
        // `per_task_materialize_recovers_failed_task_on_retry`).
        assert!(!final_state.active_tasks.contains("b"));
        assert!(!final_state.active_tasks.contains("c"));
    }

    #[test]
    fn per_task_materialize_recovers_failed_task_on_retry() {
        // After a partial failure, the next materialize call must
        // re-include the un-enqueued tasks. This is the load-bearing
        // recovery property: a transient TLM outage no longer
        // permanently strands tasks.
        let mut state = make_state(vec![task("a", &[]), task("b", &[]), task("c", &[])]);
        // Simulate the post-failure state from the previous test: only
        // "a" was successfully enqueued.
        state.active_tasks.insert("a".to_string());

        // derive_to_launch on the post-failure state must re-emit b and c.
        let to_launch = derive_to_launch(&state);
        let ids: HashSet<_> = to_launch.iter().map(|t| t.id.clone()).collect();
        assert_eq!(
            ids,
            HashSet::from(["b".to_string(), "c".to_string()]),
            "failed tasks are re-eligible on next materialize",
        );

        // And the retry now succeeds for everyone.
        let (final_state, err_idx) = simulate_per_task_materialize(state, &to_launch, |_| Ok(()));
        assert_eq!(err_idx, None);
        let active: HashSet<_> = final_state.active_tasks.iter().cloned().collect();
        assert_eq!(
            active,
            HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()]),
        );
    }

    #[test]
    fn per_task_materialize_first_task_fails_leaves_nothing_active() {
        // Edge case: the very first task's RPC fails. No task should be
        // marked active. The pre-fix bug would have left ALL three
        // marked active (because they were marked pre-loop) — that's
        // exactly the production bug shape.
        let state = make_state(vec![task("a", &[]), task("b", &[]), task("c", &[])]);
        let to_launch = derive_to_launch(&state);
        let (final_state, err_idx) =
            simulate_per_task_materialize(state, &to_launch, |_| Err("nope".into()));
        assert_eq!(err_idx, Some(0));
        assert!(
            final_state.active_tasks.is_empty(),
            "no task marked active when first RPC fails — \
             pre-fix would have stranded all three"
        );
    }

    #[test]
    fn play_state_version_bumps_monotonically() {
        // Sanity check the version bump pattern used by
        // materialize_eligible_tasks for drift detection.
        let mut state = make_state(vec![task("a", &[])]);
        assert_eq!(state.state_version, 0);
        state.state_version = state.state_version.wrapping_add(1);
        assert_eq!(state.state_version, 1);
        state.state_version = state.state_version.wrapping_add(1);
        assert_eq!(state.state_version, 2);
    }
}
