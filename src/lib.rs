use worker::*;

mod models;

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let router = Router::new();

    router
        // health
        .get("/health", |_, _| Response::ok("ok"))
        // runs (WS2 domain entity)
        .post_async("/v1/runs", |mut req, _ctx| async move {
            let body: models::CreateRun = req.json().await?;
            Response::from_json(&models::RunCreated {
                id: generate_id(),
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
            Response::from_json(&models::EventAck {
                id: generate_id(),
                event_type: body.event_type,
                accepted: true,
            })
        })
        // artifacts (WS2/WS5)
        .put_async("/v1/artifacts/:key", |mut req, ctx| async move {
            let key = ctx.param("key").unwrap().to_string();
            let data = req.bytes().await?;
            Response::from_json(&serde_json::json!({
                "key": key,
                "size": data.len(),
                "stored": true,
            }))
        })
        .get_async("/v1/artifacts/:key", |_req, ctx| async move {
            let _key = ctx.param("key").unwrap().to_string();
            Response::error("not found", 404)
        })
        // policy check (WS4)
        .post_async("/v1/policies/check", |mut req, _ctx| async move {
            let body: models::PolicyCheckRequest = req.json().await?;
            Response::from_json(&models::PolicyCheckResponse {
                action: body.action,
                decision: "allow".into(),
                reason: "no policy restrictions configured".into(),
            })
        })
        .run(req, env)
        .await
}

fn generate_id() -> String {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).unwrap();
    hex::encode(buf)
}
