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
                    let play_task_id = task_id.split('-').last().unwrap_or(&task_id).to_string();
                    
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
