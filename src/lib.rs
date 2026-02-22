use serde::Serialize;
use worker::*;
use futures_util::StreamExt;

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
            let _ = (&body.trigger, &body.metadata);
            Response::from_json(&models::RunCreated {
                id: generate_id()?,
                status: "created".into(),
                repo: body.repo,
            })
        })
        .get("/v1/runs", |_, _| {
            Response::from_json(&serde_json::json!({ "runs": [] }))
        })
        // provenance events (WS3)
        .post_async("/v1/events", |mut req, _ctx| async move {
            let body: models::ProvenanceEvent = req.json().await?;
            let _ = (&body.run_id, &body.actor, &body.payload);
            Response::from_json(&models::EventAck {
                id: generate_id()?,
                event_type: body.event_type,
                accepted: true,
            })
        })
        // artifacts (WS2/WS5)
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
            let mut size = 0usize;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                size = size
                    .checked_add(chunk.len())
                    .ok_or_else(|| Error::RustError("artifact size overflow".into()))?;
                if size > MAX_ARTIFACT_BYTES {
                    return Response::error("artifact exceeds max size", 413);
                }
            }

            Response::from_json(&serde_json::json!({
                "key": key,
                "size": size,
                "stored": true,
            }))
        })
        .get_async("/v1/artifacts/:key", |_req, ctx| async move {
            let Some(_key) = ctx.param("key").map(ToString::to_string) else {
                return Response::error("missing artifact key", 400);
            };
            Response::error("not found", 404)
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

fn generate_id() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)
        .map_err(|err| Error::RustError(format!("failed to generate id: {err}")))?;
    Ok(hex::encode(buf))
}
