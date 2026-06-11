use crate::models::{AgentTask, PlayDefinition};
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

#[derive(Serialize, Deserialize, Debug)]
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

                let storage = self.state.storage();

                let run_id = crate::generate_id().unwrap_or_else(|_| "err".to_string());

                let state = PlayState {
                    definition: def.clone(),
                    run_id: run_id.clone(),
                    tenant_id: tenant_id.clone(),
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
        let mut to_launch = Vec::new();

        for task_def in &state.definition.tasks {
            if state.completed_tasks.contains(&task_def.id) || state.active_tasks.contains(&task_def.id) {
                continue;
            }

            // Check dependencies
            let all_deps_met = task_def.depends_on.iter().all(|dep_id| state.completed_tasks.contains(dep_id));

            if all_deps_met {
                to_launch.push(task_def.clone());
            }
        }

        if to_launch.is_empty() {
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
                tenant_id: Some(state.tenant_id.clone()),
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
