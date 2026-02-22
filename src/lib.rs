use serde::Serialize;
use worker::*;

mod db;
mod models;
mod storage;

#[derive(Serialize)]
struct HealthResponse<'a> {
    service: &'a str,
    status: &'a str,
    mission: &'a str,
}

const MAX_ARTIFACT_BYTES: usize = 10 * 1024 * 1024;

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();

    let router = Router::new();

    router
        // ── Health ──────────────────────────────────────────────
        .get("/", |_, _| Response::ok("data-fabric-worker online"))
        .get("/health", |_, _| {
            Response::from_json(&HealthResponse {
                service: "data-fabric",
                status: "ok",
                mission: "velocity-for-autonomous-agent-builders",
            })
        })
        // ── Runs (WS2) ────────────────────────────────────────
        .post_async("/v1/runs", |mut req, _ctx| async move {
            let body: models::CreateRun = req.json().await?;
            let _ = (&body.trigger, &body.actor, &body.metadata);
            Response::from_json(&models::Created {
                id: generate_id()?,
                status: "created".into(),
            })
        })
        .get("/v1/runs", |_, _| {
            Response::from_json(&serde_json::json!({ "runs": [] }))
        })
        // ── WS2 domain stubs (plans, tool-calls, releases) ────
        .post_async("/v1/plans", |mut req, _ctx| async move {
            let body: models::CreatePlan = req.json().await?;
            let _ = (&body.run_id, &body.task_ids, &body.metadata);
            Response::from_json(&models::Created {
                id: generate_id()?,
                status: "created".into(),
            })
        })
        .post_async("/v1/tool-calls", |mut req, _ctx| async move {
            let body: models::RecordToolCall = req.json().await?;
            let _ = (&body.run_id, &body.task_id, &body.output, &body.duration_ms);
            Response::from_json(&models::Created {
                id: generate_id()?,
                status: "recorded".into(),
            })
        })
        .post_async("/v1/releases", |mut req, _ctx| async move {
            let body: models::CreateRelease = req.json().await?;
            let _ = (&body.run_id, &body.artifact_ids, &body.metadata);
            Response::from_json(&models::Created {
                id: generate_id()?,
                status: "created".into(),
            })
        })
        // ── Provenance Events (WS3) ───────────────────────────
        .post_async("/v1/events", |mut req, _ctx| async move {
            let body: models::IngestEvent = req.json().await?;
            let _ = (&body.run_id, &body.actor, &body.payload);
            Response::from_json(&models::EventAck {
                id: generate_id()?,
                event_type: body.event_type,
                accepted: true,
            })
        })
        // ── Artifacts (R2-backed) ─────────────────────────────
        .put_async("/v1/artifacts/:key", |mut req, ctx| async move {
            let key = match ctx.param("key") {
                Some(k) => k.to_string(),
                None => return Response::error("missing artifact key", 400),
            };
            let data = req.bytes().await?;
            if data.len() > MAX_ARTIFACT_BYTES {
                return Response::error("artifact exceeds max size", 413);
            }
            let bucket = ctx.env.bucket("ARTIFACTS")?;
            let size = storage::put_blob(&bucket, &key, data).await?;
            Response::from_json(&serde_json::json!({
                "key": key,
                "size": size,
                "stored": true,
            }))
        })
        .get_async("/v1/artifacts/:key", |_req, ctx| async move {
            let key = match ctx.param("key") {
                Some(k) => k.to_string(),
                None => return Response::error("missing artifact key", 400),
            };
            let bucket = ctx.env.bucket("ARTIFACTS")?;
            match storage::get_blob(&bucket, &key).await? {
                Some(data) => {
                    let headers = Headers::new();
                    headers.set("content-type", "application/octet-stream")?;
                    Ok(Response::from_bytes(data)?.with_headers(headers))
                }
                None => Response::error("not found", 404),
            }
        })
        // ── Policy Check (WS4) ────────────────────────────────
        .post_async("/v1/policies/check", |mut req, _ctx| async move {
            let body: models::PolicyCheckRequest = req.json().await?;
            let _ = (&body.actor, &body.resource, &body.context);
            Response::from_json(&models::PolicyCheckResponse {
                action: body.action,
                decision: "allow".into(),
                reason: "no policy restrictions configured".into(),
            })
        })
        // ── Agent Tasks (M1) ──────────────────────────────────
        .post_async("/v1/tasks", |mut req, ctx| async move {
            let body: models::CreateAgentTask = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_task(&d1, &id, &body).await?;
            Response::from_json(&models::TaskCreated {
                id,
                status: "pending".into(),
            })
        })
        .get_async("/mcp/task/next", |req, ctx| async move {
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

            let agent_id = match params.get("agent_id") {
                Some(id) => id.clone(),
                None => return Response::error("agent_id required", 400),
            };
            let caps_str = params.get("cap").cloned().unwrap_or_default();
            let caps: Vec<&str> = if caps_str.is_empty() {
                vec![]
            } else {
                caps_str.split(',').collect()
            };

            let d1 = ctx.env.d1("DB")?;
            match db::claim_next_task(&d1, &agent_id, &caps).await? {
                Some(task) => Response::from_json(&task),
                None => Ok(Response::empty()?.with_status(204)),
            }
        })
        .post_async("/mcp/task/:id/heartbeat", |req, ctx| async move {
            let task_id = match ctx.param("id") {
                Some(id) => id.to_string(),
                None => return Response::error("missing task id", 400),
            };
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let agent_id = match params.get("agent_id") {
                Some(id) => id.clone(),
                None => return Response::error("agent_id required", 400),
            };

            let d1 = ctx.env.d1("DB")?;
            let updated = db::heartbeat_task(&d1, &task_id, &agent_id).await?;
            if updated {
                Response::from_json(&serde_json::json!({ "ok": true }))
            } else {
                Response::error("task not found or not owned by agent", 404)
            }
        })
        .post_async("/mcp/task/:id/complete", |mut req, ctx| async move {
            let task_id = match ctx.param("id") {
                Some(id) => id.to_string(),
                None => return Response::error("missing task id", 400),
            };
            let body: models::TaskCompleteRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let updated = db::complete_task(&d1, &task_id, body.result.as_ref()).await?;
            if updated {
                Response::from_json(&serde_json::json!({ "status": "completed" }))
            } else {
                Response::error("task not found or not running", 404)
            }
        })
        .post_async("/mcp/task/:id/fail", |mut req, ctx| async move {
            let task_id = match ctx.param("id") {
                Some(id) => id.to_string(),
                None => return Response::error("missing task id", 400),
            };
            let body: models::TaskFailRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let new_status = db::fail_task(&d1, &task_id, &body.error).await?;
            Response::from_json(&serde_json::json!({ "status": new_status }))
        })
        // ── Agents (M1) ───────────────────────────────────────
        .post_async("/v1/agents", |mut req, ctx| async move {
            let body: models::RegisterAgent = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::register_agent(&d1, &id, &body).await?;
            Response::from_json(&serde_json::json!({
                "id": id,
                "name": body.name,
                "status": "active",
            }))
        })
        .get_async("/v1/agents", |_req, ctx| async move {
            let d1 = ctx.env.d1("DB")?;
            let agents = db::list_agents(&d1).await?;
            Response::from_json(&serde_json::json!({ "agents": agents }))
        })
        // ── Checkpoints (M2) ──────────────────────────────────
        .post_async("/v1/checkpoints", |mut req, ctx| async move {
            let body: models::CreateCheckpoint = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let bucket = ctx.env.bucket("ARTIFACTS")?;
            let id = generate_id()?;
            let r2_key = format!("checkpoints/{}/{}", body.thread_id, id);

            let state_bytes =
                serde_json::to_vec(&body.state).map_err(|e| Error::RustError(e.to_string()))?;
            let size = storage::put_blob(&bucket, &r2_key, state_bytes).await? as i64;
            db::create_checkpoint(&d1, &id, &body, &r2_key, size).await?;

            Response::from_json(&models::CheckpointCreated {
                id,
                thread_id: body.thread_id,
                state_r2_key: r2_key,
            })
        })
        .get_async(
            "/v1/checkpoints/threads/:thread_id",
            |_req, ctx| async move {
                let thread_id = ctx.param("thread_id").unwrap().to_string();
                let d1 = ctx.env.d1("DB")?;
                match db::get_latest_checkpoint(&d1, &thread_id).await? {
                    Some(row) => Response::from_json(&row.into_checkpoint()),
                    None => Response::error("no checkpoint found", 404),
                }
            },
        )
        .get_async("/v1/checkpoints/:id", |_req, ctx| async move {
            let id = ctx.param("id").unwrap().to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_checkpoint_by_id(&d1, &id).await? {
                Some(row) => Response::from_json(&row.into_checkpoint()),
                None => Response::error("checkpoint not found", 404),
            }
        })
        .delete_async("/v1/checkpoints/:id", |_req, ctx| async move {
            let id = match ctx.param("id") {
                Some(k) => k.to_string(),
                None => return Response::error("missing checkpoint id", 400),
            };
            let d1 = ctx.env.d1("DB")?;

            let row = match db::get_checkpoint_by_id(&d1, &id).await? {
                Some(r) => r,
                None => return Response::error("checkpoint not found", 404),
            };
            let r2_key = row.state_r2_key.clone();

            let deleted = db::delete_checkpoint(&d1, &id).await?;
            if deleted {
                let bucket = ctx.env.bucket("ARTIFACTS")?;
                let _ = storage::delete_blob(&bucket, &r2_key).await;
                Response::from_json(&serde_json::json!({ "deleted": true }))
            } else {
                Response::error("checkpoint not found", 404)
            }
        })
        // ── Traces / Provenance (WS3: issue #43) ──────────────
        .get_async("/v1/traces/:run_id", |_req, ctx| async move {
            let run_id = match ctx.param("run_id") {
                Some(r) => r.to_string(),
                None => return Response::error("missing run_id", 400),
            };
            let d1 = ctx.env.d1("DB")?;
            let events = db::get_trace_for_run(&d1, &run_id).await?;
            Response::from_json(&models::TraceResponse { run_id, events })
        })
        .get_async("/v1/traces/:run_id/lineage", |_req, ctx| async move {
            let run_id = match ctx.param("run_id") {
                Some(r) => r.to_string(),
                None => return Response::error("missing run_id", 400),
            };
            let d1 = ctx.env.d1("DB")?;
            let events = db::get_trace_for_run(&d1, &run_id).await?;
            Response::from_json(&models::TraceResponse { run_id, events })
        })
        // ── Graph Events (M3) ─────────────────────────────────
        .post_async("/v1/graph-events", |mut req, ctx| async move {
            let body: models::GraphEventBatch = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let now = js_sys::Date::new_0().to_iso_string().as_string().unwrap();

            let mut events: Vec<(String, &models::GraphEvent, String)> =
                Vec::with_capacity(body.events.len());
            for e in &body.events {
                let id = generate_id()?;
                events.push((id, e, now.clone()));
            }

            let count = events.len();
            db::insert_events_bronze(&d1, &events).await?;
            db::insert_events_silver(&d1, &events, &now).await?;

            Response::from_json(&models::GraphEventAck {
                accepted: count,
                queued: true,
            })
        })
        .run(req, env)
        .await
}

/// Queue consumer: processes enrichment jobs from the events queue.
/// Ack all on success; on error retry the batch.
#[event(queue)]
pub async fn queue(batch: MessageBatch<serde_json::Value>, _env: Env, _ctx: Context) -> Result<()> {
    let queue_name = batch.queue();
    let messages = batch.messages()?;
    for msg in messages {
        let _body = msg.body();
        worker::console_log!("[queue {}] processing message", queue_name);
    }
    batch.ack_all();
    Ok(())
}

fn generate_id() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)
        .map_err(|err| Error::RustError(format!("failed to generate id: {err}")))?;
    Ok(hex::encode(buf))
}
