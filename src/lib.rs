use futures_util::StreamExt;
use serde::Serialize;
use worker::*;

mod models;

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
        // health
        .get("/", |_, _| Response::ok("data-fabric-worker online"))
        .get("/health", |_, _| {
            Response::from_json(&HealthResponse {
                service: "data-fabric",
                status: "ok",
                mission: "velocity-for-autonomous-agent-builders",
            })
        })
        // runs (WS2 domain entity)
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
        // tasks (WS2)
        .post_async("/v1/tasks", |mut req, _ctx| async move {
            let body: models::CreateTask = req.json().await?;
            let _ = (&body.run_id, &body.plan_id, &body.actor, &body.metadata);
            Response::from_json(&models::Created {
                id: generate_id()?,
                status: "created".into(),
            })
        })
        // plans (WS2)
        .post_async("/v1/plans", |mut req, _ctx| async move {
            let body: models::CreatePlan = req.json().await?;
            let _ = (&body.run_id, &body.task_ids, &body.metadata);
            Response::from_json(&models::Created {
                id: generate_id()?,
                status: "created".into(),
            })
        })
        // tool calls (WS2)
        .post_async("/v1/tool-calls", |mut req, _ctx| async move {
            let body: models::RecordToolCall = req.json().await?;
            let _ = (&body.run_id, &body.task_id, &body.output, &body.duration_ms);
            Response::from_json(&models::Created {
                id: generate_id()?,
                status: "recorded".into(),
            })
        })
        // releases (WS2)
        .post_async("/v1/releases", |mut req, _ctx| async move {
            let body: models::CreateRelease = req.json().await?;
            let _ = (&body.run_id, &body.artifact_ids, &body.metadata);
            Response::from_json(&models::Created {
                id: generate_id()?,
                status: "created".into(),
            })
        })
        // provenance events (WS3)
        .post_async("/v1/events", |mut req, _ctx| async move {
            let body: models::IngestEvent = req.json().await?;
            let _ = (&body.run_id, &body.actor, &body.payload);
            Response::from_json(&models::EventAck {
                id: generate_id()?,
                event_type: body.event_type,
                accepted: true,
            })
        })
        // artifacts (WS2/WS5) — R2 write/read path
        .put_async("/v1/artifacts/:key", |mut req, ctx| async move {
            let Some(key) = ctx.param("key").map(ToString::to_string) else {
                return Response::error("missing artifact key", 400);
            };

            if let Some(value) = req.headers().get("content-length")? {
                let content_length = match value.parse::<usize>() {
                    Ok(parsed) => parsed,
                    Err(_) => return Response::error("invalid content-length header", 400),
                };
                if content_length > MAX_ARTIFACT_BYTES {
                    return Response::error("artifact exceeds max size", 413);
                }
            }

            let mut stream = req.stream()?;
            let mut data = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if data.len().saturating_add(chunk.len()) > MAX_ARTIFACT_BYTES {
                    return Response::error("artifact exceeds max size", 413);
                }
                data.extend_from_slice(&chunk);
            }
            let size = data.len();

            let bucket = match ctx.env.bucket("ARTIFACTS") {
                Ok(b) => b,
                Err(_) => return Response::error("R2 not configured", 503),
            };
            bucket.put(&key, data).execute().await?;

            Response::from_json(&serde_json::json!({
                "key": key,
                "size": size,
                "stored": true,
            }))
        })
        .get_async("/v1/artifacts/:key", |_req, ctx| async move {
            let Some(key) = ctx.param("key").map(ToString::to_string) else {
                return Response::error("missing artifact key", 400);
            };
            let bucket = match ctx.env.bucket("ARTIFACTS") {
                Ok(b) => b,
                Err(_) => return Response::error("R2 not configured", 503),
            };
            match bucket.get(&key).execute().await? {
                Some(obj) => {
                    let body = obj.body().ok_or_else(|| Error::RustError("no body".into()))?;
                    let bytes = body.bytes().await?;
                    Response::from_bytes(bytes)
                }
                None => Response::error("not found", 404),
            }
        })
        // policy check (WS4)
        .post_async("/v1/policies/check", |mut req, _ctx| async move {
            let body: models::PolicyCheckRequest = req.json().await?;
            let _ = (&body.actor, &body.resource, &body.context);
            Response::from_json(&models::PolicyCheckResponse {
                action: body.action,
                decision: "allow".into(),
                reason: "no policy restrictions configured".into(),
            })
        })
        .run(req, env)
        .await
}

/// Queue consumer: processes enrichment jobs from the events queue.
/// Ack all on success; on error retry the batch.
#[event(queue)]
pub async fn queue(
    batch: MessageBatch<serde_json::Value>,
    _env: Env,
    _ctx: Context,
) -> Result<()> {
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
