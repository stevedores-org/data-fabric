use crate::models::{Checkpoint, CreateCheckpoint};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use worker::*;

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug)]
struct ThreadState {
    latest_checkpoint: Option<Checkpoint>,
    history: VecDeque<Checkpoint>,
}

#[durable_object]
pub struct ThreadManager {
    state: State,
    #[allow(dead_code)]
    env: Env,
}

impl DurableObject for ThreadManager {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let method = req.method();

        match (method, path.as_str()) {
            (Method::Post, "/checkpoint") => {
                let body: CreateCheckpoint = req.json().await?;
                let storage = self.state.storage();
                
                let id = crate::generate_id().unwrap_or_else(|_| "err".to_string());
                let now = js_sys::Date::now() as u64;
                let created_at = js_sys::Date::new(&serde_wasm_bindgen::to_value(&now).unwrap()).to_iso_string().as_string().unwrap();

                let checkpoint = Checkpoint {
                    id: id.clone(),
                    thread_id: body.thread_id.clone(),
                    node_id: body.node_id.clone(),
                    parent_id: body.parent_id.clone(),
                    state_r2_key: format!("threads/{}/{}", body.thread_id, id), // We'll still use R2 for large blobs if needed, but DO can store small states
                    state_size_bytes: Some(serde_json::to_string(&body.state).unwrap_or_default().len() as i64),
                    metadata: body.metadata.clone(),
                    created_at,
                };

                // Store state in DO storage directly for fast access
                // If state is too large (> 128KB), we should probably fail or use R2
                storage.put(&format!("state:{}", id), &body.state).await?;
                storage.put("latest", &checkpoint).await?;

                let mut history: VecDeque<Checkpoint> = storage.get("history").await.ok().flatten().unwrap_or_default();
                history.push_front(checkpoint.clone());
                if history.len() > 50 {
                    history.pop_back();
                }
                storage.put("history", history).await?;

                Response::from_json(&checkpoint)
            }
            (Method::Get, "/latest") => {
                let storage = self.state.storage();
                let latest: Option<Checkpoint> = storage.get("latest").await.ok().flatten();
                
                if let Some(cp) = latest {
                    let state: Option<serde_json::Value> = storage.get(&format!("state:{}", cp.id)).await.ok().flatten();
                    // We need a way to return the state too. 
                    // Let's wrap it in a response that includes the state.
                    Response::from_json(&serde_json::json!({
                        "checkpoint": cp,
                        "state": state
                    }))
                } else {
                    Ok(Response::empty()?.with_status(404))
                }
            }
            (Method::Get, "/history") => {
                let storage = self.state.storage();
                let history: VecDeque<Checkpoint> = storage.get("history").await.ok().flatten().unwrap_or_default();
                Response::from_json(&history)
            }
            _ => Response::error("not found", 404),
        }
    }
}
