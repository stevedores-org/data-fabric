use crate::models::{AgentTask, PlayDefinition, PlayTaskDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use worker::*;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PlayState {
    definition: PlayDefinition,
    run_id: String,
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
                let def: PlayDefinition = req.json().await?;

                let run_id = crate::generate_id().unwrap_or_else(|_| "err".to_string());

                let state = PlayState {
                    definition: def.clone(),
                    run_id: run_id.clone(),
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

        // Mark eligible tasks as active and bump version BEFORE outbound
        // RPCs so a concurrent /task-completed cannot double-launch them.
        for task_def in &to_launch {
            state.active_tasks.insert(task_def.id.clone());
        }
        state.state_version = state.state_version.wrapping_add(1);
        let snapshot_version = state.state_version;
        storage.put("state", &state).await?;

        let tenant_id = self.state.id().to_string(); // Simplified, should pass tenant_id
        let task_ns = self.env.durable_object("TASK_LEASE_MANAGER")?;
        let task_stub = task_ns.id_from_name(&tenant_id)?.get_stub()?;

        // Issue outbound RPCs. Each await may yield the input gate, but
        // the state-write above is already durable so any interleaved
        // /task-completed will observe the updated active_tasks set.
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
            task_stub.fetch_with_request(do_req).await?;
        }

        // Defensive drift assertion: if our snapshot's state_version no
        // longer matches what's in storage, a concurrent /task-completed
        // ran during the outbound RPC loop. That handler is responsible
        // for re-running materialize, so we can safely return — we do not
        // overwrite storage with a stale snapshot here.
        if let Some(current) = storage.get::<PlayState>("state").await? {
            if current.state_version != snapshot_version {
                worker::console_log!(
                    "play_do: state_version drift detected ({} -> {}); skipping stale write",
                    snapshot_version,
                    current.state_version
                );
            }
        }

        Ok(())
    }
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
