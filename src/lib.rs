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
        // ── Runs (WS2, D1-backed) ────────────────────────────
        .post_async("/v1/runs", |mut req, ctx| async move {
            let body: models::CreateRun = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_run(&d1, &id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "created".into(),
            })
        })
        .get_async("/v1/runs", |req, ctx| async move {
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let repo = params.get("repo").map(|s| s.as_str());
            let limit = params
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(50u32)
                .min(200);
            let d1 = ctx.env.d1("DB")?;
            let runs = db::list_runs(&d1, repo, limit).await?;
            Response::from_json(&serde_json::json!({ "runs": runs }))
        })
        .get_async("/v1/runs/:id", |_req, ctx| async move {
            let id = ctx.param("id").unwrap().to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_run(&d1, &id).await? {
                Some(run) => Response::from_json(&run),
                None => Response::error("run not found", 404),
            }
        })
        // ── WS2 Tasks (run-scoped, D1-backed) ───────────────
        .post_async("/v1/runs/:run_id/tasks", |mut req, ctx| async move {
            let run_id = ctx.param("run_id").unwrap().to_string();
            let body: models::CreateTask = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_ws2_task(&d1, &id, &run_id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "created".into(),
            })
        })
        .get_async("/v1/runs/:run_id/tasks", |_req, ctx| async move {
            let run_id = ctx.param("run_id").unwrap().to_string();
            let d1 = ctx.env.d1("DB")?;
            let tasks = db::list_ws2_tasks(&d1, &run_id).await?;
            Response::from_json(&serde_json::json!({ "tasks": tasks }))
        })
        // ── Plans (WS2, D1-backed) ──────────────────────────
        .post_async("/v1/plans", |mut req, ctx| async move {
            let body: models::CreatePlan = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_plan(&d1, &id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "created".into(),
            })
        })
        // ── Tool Calls (WS2, D1-backed) ─────────────────────
        .post_async("/v1/tool-calls", |mut req, ctx| async move {
            let body: models::RecordToolCall = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::record_tool_call(&d1, &id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "recorded".into(),
            })
        })
        // ── Releases (WS2, D1-backed) ───────────────────────
        .post_async("/v1/releases", |mut req, ctx| async move {
            let body: models::CreateRelease = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_release(&d1, &id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "created".into(),
            })
        })
        // ── Provenance Events (WS3, D1-backed) ──────────────
        .post_async("/v1/events", |mut req, ctx| async move {
            let body: models::IngestEvent = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::ingest_event(&d1, &id, &body).await?;
            Response::from_json(&models::EventAck {
                id,
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
        // ── Policy Check (WS4, D1-backed) ──────────────────────
        .post_async("/v1/policies/check", |mut req, ctx| async move {
            let body: models::PolicyCheckRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            let decision = "allow";
            let reason = "no policy restrictions configured";
            db::record_policy_check(&d1, &id, &body, decision, reason).await?;
            Response::from_json(&models::PolicyCheckResponse {
                action: body.action,
                decision: decision.into(),
                reason: reason.into(),
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
        .get_async("/v1/traces/:run_id", |req, ctx| async move {
            let run_id = match ctx.param("run_id") {
                Some(r) => r.to_string(),
                None => return Response::error("missing run_id", 400),
            };
            let limit =
                parse_limit_query(req.url().ok(), "limit").unwrap_or(db::TRACE_DEFAULT_LIMIT);
            let d1 = ctx.env.d1("DB")?;
            let total = db::count_trace_events_for_run(&d1, &run_id).await?;
            let events = db::get_trace_for_run(&d1, &run_id, limit).await?;
            let truncated = total > events.len() as u32;
            Response::from_json(&models::TraceResponse {
                run_id,
                events,
                total: Some(total as usize),
                truncated: Some(truncated),
            })
        })
        .get_async("/v1/traces/:run_id/lineage", |req, ctx| async move {
            let run_id = match ctx.param("run_id") {
                Some(r) => r.to_string(),
                None => return Response::error("missing run_id", 400),
            };
            let hops = parse_limit_query(req.url().ok(), "hops").unwrap_or(5);
            let d1 = ctx.env.d1("DB")?;
            let edges = db::get_provenance_chain(&d1, "run", &run_id, "forward", hops).await?;
            Response::from_json(&models::ProvenanceResponse {
                entity_kind: "run".into(),
                entity_id: run_id,
                direction: "forward".into(),
                hops,
                edges,
            })
        })
        // ── Provenance Chain (WS3) ──────────────────────────────
        .get_async("/v1/provenance/:kind/:id", |req, ctx| async move {
            let kind = match ctx.param("kind") {
                Some(k) => k.to_string(),
                None => return Response::error("missing kind", 400),
            };
            let id = match ctx.param("id") {
                Some(i) => i.to_string(),
                None => return Response::error("missing id", 400),
            };
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let direction = params
                .get("direction")
                .map(|s| s.as_str())
                .unwrap_or("forward");
            let hops = params
                .get("hops")
                .and_then(|s| s.parse().ok())
                .unwrap_or(5u32)
                .min(100);
            let d1 = ctx.env.d1("DB")?;
            let edges = db::get_provenance_chain(&d1, &kind, &id, direction, hops).await?;
            Response::from_json(&models::ProvenanceResponse {
                entity_kind: kind,
                entity_id: id,
                direction: direction.into(),
                hops,
                edges,
            })
        })
        // ── Gold Layer: Run Summaries (WS3) ─────────────────────
        .get_async("/v1/runs/:run_id/summary", |_req, ctx| async move {
            let run_id = ctx.param("run_id").unwrap().to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_run_summary(&d1, &run_id).await? {
                Some(summary) => Response::from_json(&summary),
                None => Response::error("run summary not found", 404),
            }
        })
        .get_async("/v1/gold/run-summaries", |req, ctx| async move {
            let limit = parse_limit_query(req.url().ok(), "limit")
                .unwrap_or(50)
                .min(200);
            let d1 = ctx.env.d1("DB")?;
            let summaries = db::list_run_summaries(&d1, limit).await?;
            Response::from_json(&serde_json::json!({ "summaries": summaries }))
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

            let mut silver_events: Vec<(String, String, &models::GraphEvent, String)> =
                Vec::with_capacity(events.len());
            for (bronze_id, evt, created_at) in &events {
                let silver_id = generate_id()?;
                silver_events.push((silver_id, bronze_id.clone(), evt, created_at.clone()));
            }
            db::insert_events_silver(&d1, &silver_events, &now).await?;

            Response::from_json(&models::GraphEventAck {
                accepted: count,
                queued: true,
            })
        })
        .run(req, env)
        .await
}

/// Queue consumer: enriches events — silver promotion, causality edges, gold summaries.
#[event(queue)]
pub async fn queue(batch: MessageBatch<serde_json::Value>, env: Env, _ctx: Context) -> Result<()> {
    let queue_name = batch.queue();
    let d1 = env.d1("DB")?;
    let messages = batch.messages()?;

    for msg in &messages {
        let body = msg.body();
        match serde_json::from_value::<models::GraphEvent>(body.clone()) {
            Ok(evt) => {
                let now = js_sys::Date::new_0().to_iso_string().as_string().unwrap();

                // Silver promotion
                let bronze_id = generate_id()?;
                let silver_id = generate_id()?;
                let silver_events = vec![(silver_id, bronze_id.clone(), &evt, now.clone())];
                if let Err(e) = db::insert_events_silver(&d1, &silver_events, &now).await {
                    worker::console_log!("[queue {}] silver promotion error: {}", queue_name, e);
                }

                // Causality edges
                if let Err(e) = db::insert_causality_from_event(&d1, &evt).await {
                    worker::console_log!("[queue {}] causality insert error: {}", queue_name, e);
                }

                // Gold layer summary
                if let Some(ref run_id) = evt.run_id {
                    if let Err(e) = db::upsert_run_summary(
                        &d1,
                        run_id,
                        evt.actor.as_deref(),
                        &evt.event_type,
                        &now,
                    )
                    .await
                    {
                        worker::console_log!(
                            "[queue {}] run summary upsert error: {}",
                            queue_name,
                            e
                        );
                    }
                }

                msg.ack();
            }
            Err(e) => {
                worker::console_log!(
                    "[queue {}] failed to deserialize message: {}",
                    queue_name,
                    e
                );
                msg.retry();
            }
        }
    }
    Ok(())
}

fn generate_id() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)
        .map_err(|err| Error::RustError(format!("failed to generate id: {err}")))?;
    Ok(hex::encode(buf))
}

/// Parse a numeric query param (e.g. limit, hops). Returns None if URL is None or param missing/invalid.
fn parse_limit_query(url: Option<worker::Url>, param: &str) -> Option<u32> {
    let url = url?;
    let value = url.query_pairs().find(|(k, _)| k == param)?.1;
    value.parse().ok().filter(|&n| n > 0 && n <= 10_000)
}
