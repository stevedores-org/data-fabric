use crate::models::{AgentTask, PlayDefinition, PlayTaskDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use worker::*;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct PlayState {
    pub(crate) definition: PlayDefinition,
    pub(crate) run_id: String,
    pub(crate) completed_tasks: HashSet<String>,
    pub(crate) active_tasks: HashSet<String>,
}

// ── Pure state-machine helpers ──────────────────────────────────────────
//
// These free functions mirror the DAG-traversal logic that lives inside
// `materialize_eligible_tasks`. They're exposed so native unit tests can
// exercise the eligibility rules without standing up DO storage.

/// Compute the list of `PlayTaskDefinition`s eligible to be materialized
/// right now:
///   * not already in `completed_tasks`
///   * not already in `active_tasks`
///   * every dep in `depends_on` is present in `completed_tasks`
pub(crate) fn eligible_tasks(state: &PlayState) -> Vec<PlayTaskDefinition> {
    let mut out = Vec::new();
    for task_def in &state.definition.tasks {
        if state.completed_tasks.contains(&task_def.id)
            || state.active_tasks.contains(&task_def.id)
        {
            continue;
        }
        let all_deps_met = task_def
            .depends_on
            .iter()
            .all(|dep_id| state.completed_tasks.contains(dep_id));
        if all_deps_met {
            out.push(task_def.clone());
        }
    }
    out
}

/// Apply a `/task-completed` event idempotently. The same `task_id` reported
/// twice is a no-op the second time. Returns `true` if the state changed.
/// PR C will wire this into the `/task-completed` handler in place of the
/// current inlined `insert`/`remove` pair.
#[allow(dead_code)]
pub(crate) fn mark_completed(state: &mut PlayState, task_id: &str) -> bool {
    let inserted = state.completed_tasks.insert(task_id.to_string());
    let removed = state.active_tasks.remove(task_id);
    inserted || removed
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
                let storage = self.state.storage();

                let run_id = crate::generate_id().unwrap_or_else(|_| "err".to_string());

                let state = PlayState {
                    definition: def.clone(),
                    run_id: run_id.clone(),
                    completed_tasks: HashSet::new(),
                    active_tasks: HashSet::new(),
                };

                storage.put("state", &state).await?;

                // Materialize initial tasks
                self.materialize_eligible_tasks(&state).await?;

                Response::from_json(&serde_json::json!({
                    "run_id": run_id,
                    "status": "launched"
                }))
            }
            (Method::Post, "/task-completed") => {
                let task_id: String = req.json().await?;
                let storage = self.state.storage();
                let mut state: PlayState = storage.get("state").await?.ok_or_else(|| Error::RustError("state not found".into()))?;

                state.completed_tasks.insert(task_id.clone());
                state.active_tasks.remove(&task_id);

                storage.put("state", &state).await?;

                // Materialize next batch of tasks
                self.materialize_eligible_tasks(&state).await?;

                Response::ok("ok")
            }
            _ => Response::error("not found", 404),
        }
    }
}

impl PlayManager {
    async fn materialize_eligible_tasks(&self, state: &PlayState) -> Result<()> {
        let to_launch = eligible_tasks(state);

        if to_launch.is_empty() {
            return Ok(());
        }

        let tenant_id = self.state.id().to_string(); // Simplified, should pass tenant_id
        let task_ns = self.env.durable_object("TASK_LEASE_MANAGER")?;
        let task_stub = task_ns.id_from_name(&tenant_id)?.get_stub()?;

        let storage = self.state.storage();
        let mut current_state: PlayState = storage.get("state").await?.ok_or_else(|| Error::RustError("state not found".into()))?;

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
                parent_task_id: None, // Could map to first dependency
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
                    body: Some(serde_wasm_bindgen::to_value(&task).map_err(|e| Error::RustError(e.to_string()))?),
                    ..Default::default()
                }
            )?;
            task_stub.fetch_with_request(do_req).await?;

            current_state.active_tasks.insert(task_def.id.clone());
        }

        storage.put("state", &current_state).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the DAG-traversal helpers extracted from PlayManager.

    use super::*;
    use crate::models::{PlayDefinition, PlayTaskDefinition};
    use std::collections::HashSet;

    fn task(id: &str, deps: &[&str]) -> PlayTaskDefinition {
        PlayTaskDefinition {
            id: id.to_string(),
            task_type: "build".to_string(),
            priority: 0,
            params: None,
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn three_task_dag() -> PlayDefinition {
        // root -> child_a -> grandchild
        //     \-> child_b ----^
        PlayDefinition {
            name: "play".to_string(),
            goal: "test".to_string(),
            tasks: vec![
                task("root", &[]),
                task("child_a", &["root"]),
                task("child_b", &["root"]),
                task("grandchild", &["child_a", "child_b"]),
            ],
        }
    }

    fn fresh_state(def: PlayDefinition) -> PlayState {
        PlayState {
            definition: def,
            run_id: "run-1".to_string(),
            completed_tasks: HashSet::new(),
            active_tasks: HashSet::new(),
        }
    }

    // ── launch / initial materialization ───────────────────────────

    #[test]
    fn launch_with_three_task_dag_materializes_only_root() {
        // Use a 3-task chain (root -> middle -> leaf) so the "3-task DAG"
        // language from the PR brief is reflected literally.
        let def = PlayDefinition {
            name: "chain".to_string(),
            goal: "linear".to_string(),
            tasks: vec![
                task("root", &[]),
                task("middle", &["root"]),
                task("leaf", &["middle"]),
            ],
        };
        let state = fresh_state(def);

        let eligible = eligible_tasks(&state);
        assert_eq!(eligible.len(), 1, "only root should be eligible at launch");
        assert_eq!(eligible[0].id, "root");
    }

    // ── task completion fans out ───────────────────────────────────

    #[test]
    fn task_completed_for_root_materializes_eligible_children() {
        let mut state = fresh_state(three_task_dag());

        // Pretend the root was launched and finished.
        state.active_tasks.insert("root".to_string());
        assert!(mark_completed(&mut state, "root"));

        let eligible = eligible_tasks(&state);
        let mut ids: Vec<_> = eligible.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["child_a", "child_b"]);
    }

    #[test]
    fn task_completed_with_unsatisfied_deps_does_not_materialize_grandchild() {
        let mut state = fresh_state(three_task_dag());
        state.completed_tasks.insert("root".to_string());
        // child_a is done, child_b is still in flight
        state.active_tasks.insert("child_b".to_string());
        mark_completed(&mut state, "child_a");

        let eligible = eligible_tasks(&state);
        let ids: Vec<_> = eligible.iter().map(|t| t.id.as_str()).collect();
        assert!(
            !ids.contains(&"grandchild"),
            "grandchild requires both child_a AND child_b — must NOT be eligible"
        );
        // And nothing else should sneak in either — child_a is done, child_b
        // is active, root is done.
        assert!(eligible.is_empty(), "no tasks should be eligible: {ids:?}");
    }

    // ── idempotency ────────────────────────────────────────────────

    #[test]
    fn task_completed_is_idempotent_no_duplicate_state() {
        let mut state = fresh_state(three_task_dag());
        state.active_tasks.insert("root".to_string());

        let first = mark_completed(&mut state, "root");
        let second = mark_completed(&mut state, "root");

        assert!(first, "first /task-completed should change state");
        assert!(
            !second,
            "second /task-completed for same id must be a no-op"
        );
        assert_eq!(state.completed_tasks.len(), 1);
        assert!(state.completed_tasks.contains("root"));
        assert!(!state.active_tasks.contains("root"));
    }

    #[test]
    fn already_active_or_completed_tasks_are_skipped_by_eligibility() {
        let mut state = fresh_state(three_task_dag());
        // Mark root active so it should NOT re-appear as eligible.
        state.active_tasks.insert("root".to_string());

        let eligible = eligible_tasks(&state);
        assert!(eligible.is_empty(), "root is active, nothing else has deps met");

        // Now mark root completed; child_a / child_b become eligible.
        state.active_tasks.remove("root");
        state.completed_tasks.insert("root".to_string());
        let eligible = eligible_tasks(&state);
        let mut ids: Vec<_> = eligible.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["child_a", "child_b"]);

        // Pretend child_a was materialized (now active). It should drop out.
        state.active_tasks.insert("child_a".to_string());
        let eligible = eligible_tasks(&state);
        let ids: Vec<_> = eligible.iter().map(|t| t.id.clone()).collect();
        assert_eq!(ids, vec!["child_b"]);
    }
}
