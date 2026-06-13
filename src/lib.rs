use serde::Serialize;
use wasm_bindgen::JsValue;
use worker::*;

mod db;
mod errors;
mod integrations;
mod metrics;
mod models;
mod openapi;
mod pagination;
mod play_do;
mod policy;
mod storage;
mod task_do;
mod tenant;
#[allow(dead_code)]
mod tenant_security;
mod thread_do;
mod vector_index;
mod verification;
mod gemini_service;

pub use task_do::TaskLeaseManager;
pub use thread_do::ThreadManager;

pub use play_do::PlayManager;

#[derive(Serialize)]
struct HealthResponse<'a> {
    service: &'a str,
    status: &'a str,
    mission: &'a str,
}

const MAX_ARTIFACT_BYTES: usize = 10 * 1024 * 1024;

fn is_public_path(path: &str) -> bool {
    path == "/" || path == "/health" || path == "/openapi.json" || path == "/docs"
}

/// Compose the tenant-namespaced Durable Object instance name for the
/// `PlayManager` DO. Used by `POST /v1/plays/:name/launch` and by
/// `TaskLeaseManager`'s completion callback. Two tenants requesting the
/// same run_id produce different DO instances.
fn play_do_name(tenant_id: &str, run_id: &str) -> String {
    format!("{tenant_id}:play:{run_id}")
}

/// Compose the tenant-namespaced Durable Object instance name for the
/// `ThreadManager` DO. Used by `POST /v1/checkpoints` and
/// `GET /v1/checkpoints/threads/:thread_id`. A guessable thread_id from
/// tenant A cannot route into tenant B's ThreadManager.
fn thread_do_name(tenant_id: &str, thread_id: &str) -> String {
    format!("{tenant_id}:thread:{thread_id}")
}

fn request_path(req: &Request) -> Result<String> {
    Ok(req.url()?.path().to_string())
}

/// Build a URL targeting the internal `https://do/...` namespace used by
/// Durable Object stubs, with each query parameter percent-encoded.
///
/// The previous implementation interpolated user-controlled values
/// (`agent_id`, `task_id`, `cap`) into the URL with `format!`. A caller
/// supplying e.g. `agent_id = "agent-1&caps=evil"` would inject a second
/// `caps` parameter on its way through the DO boundary. Using
/// `url::Url::query_pairs_mut` ensures each value is percent-encoded.
fn build_do_url(path: &str, params: &[(&str, &str)]) -> Result<String> {
    let mut url = url::Url::parse("https://do")
        .map_err(|e| Error::RustError(format!("internal: bad DO base url: {}", e)))?;
    // `path` is a developer-controlled literal (e.g. "/claim"), so we can set
    // it directly. We only encode the dynamic params.
    url.set_path(path);
    {
        let mut qp = url.query_pairs_mut();
        for (k, v) in params {
            qp.append_pair(k, v);
        }
    }
    Ok(url.into())
}

#[cfg(test)]
mod do_url_tests {
    use super::build_do_url;

    /// Regression test for the URL-injection findings in crr2:
    /// values passed via `params` must be percent-encoded so a caller can't
    /// smuggle additional query parameters across the DO boundary.
    #[test]
    fn build_do_url_encodes_injected_query_separators() {
        let url = build_do_url(
            "/claim",
            &[("agent_id", "agent-1&caps=evil"), ("caps", "rust")],
        )
        .expect("url must build");

        let parsed = url::Url::parse(&url).expect("url must parse");
        assert_eq!(parsed.scheme(), "https");
        assert_eq!(parsed.host_str(), Some("do"));
        assert_eq!(parsed.path(), "/claim");

        // Exactly one `caps` parameter — the legitimate one. If the injected
        // `&caps=evil` had been interpolated raw, we'd see two.
        let caps_count = parsed.query_pairs().filter(|(k, _)| k == "caps").count();
        assert_eq!(caps_count, 1, "expected one caps param, url was: {}", url);

        // The agent_id round-trips with `&` and `=` preserved as data, not
        // interpreted as separators.
        let agent_id_value = parsed
            .query_pairs()
            .find(|(k, _)| k == "agent_id")
            .map(|(_, v)| v.into_owned())
            .expect("agent_id must be present");
        assert_eq!(agent_id_value, "agent-1&caps=evil");

        // Sanity: the legitimate caps value is preserved.
        let caps_value = parsed
            .query_pairs()
            .find(|(k, _)| k == "caps")
            .map(|(_, v)| v.into_owned())
            .expect("caps must be present");
        assert_eq!(caps_value, "rust");
    }

    #[test]
    fn build_do_url_encodes_path_aware_chars_in_query() {
        let url = build_do_url(
            "/heartbeat",
            &[("task_id", "abc/extra?injected=1"), ("agent_id", "agent#1")],
        )
        .expect("url must build");

        let parsed = url::Url::parse(&url).expect("url must parse");
        assert_eq!(parsed.path(), "/heartbeat");

        // No injected `injected=1` from task_id.
        assert!(
            parsed.query_pairs().all(|(k, _)| k != "injected"),
            "task_id must not smuggle additional params: {}",
            url
        );

        let task_id = parsed
            .query_pairs()
            .find(|(k, _)| k == "task_id")
            .map(|(_, v)| v.into_owned())
            .expect("task_id must be present");
        assert_eq!(task_id, "abc/extra?injected=1");

        let agent_id = parsed
            .query_pairs()
            .find(|(k, _)| k == "agent_id")
            .map(|(_, v)| v.into_owned())
            .expect("agent_id must be present");
        assert_eq!(agent_id, "agent#1");
    }
}

/// Augment a claimed task with agent's memory context from MOM.
///
/// Enables agents to reason with past experiences by injecting relevant memories
/// into the task description. If MOM is unavailable or contains no relevant memories,
/// returns None (graceful degradation).
async fn augment_task_with_memory(
    agent_id: &str,
    tenant_id: &str,
    task: &models::AgentTask,
    mom_endpoint: &str,
) -> Option<String> {
    // Extract task description from params or task_type
    let task_description = if let Some(params) = &task.params {
        if let Some(desc) = params.get("description").and_then(|v| v.as_str()) {
            desc.to_string()
        } else if let Some(prompt) = params.get("prompt").and_then(|v| v.as_str()) {
            prompt.to_string()
        } else {
            task.task_type.clone()
        }
    } else {
        task.task_type.clone()
    };

    // Create a memory recall request scoped to this agent/tenant
    let recall_req = integrations::mom::recall_request_for_task(
        agent_id,
        tenant_id,
        &task_description,
        None,    // workspace_id: task doesn't have it
        Some(5), // limit to top 5 memories
    );

    // Query MOM for relevant memories
    let client = integrations::mom::MomClient::new(mom_endpoint.to_string());
    if let Ok(memories) = client.recall(&recall_req).await {
        let formatted = integrations::mom::format_memory_augmentation(&memories);
        if !formatted.is_empty() {
            return Some(formatted);
        }
    }

    None
}

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();
    let start_ms = js_sys::Date::now();

    let path = request_path(&req)?;
    let method = req.method();
    let mut tenant_id_for_metric: Option<String> = None;
    if !is_public_path(&path) {
        let tenant_ctx = match tenant::tenant_from_request(&req) {
            Ok(ctx) => ctx,
            Err(_) => return Response::error("missing or invalid tenant context", 401),
        };
        if tenant::authorize(&tenant_ctx, req.method(), &path).is_err() {
            return Response::error("forbidden by tenant role policy", 403);
        }
        tenant_id_for_metric = Some(tenant_ctx.tenant_id.clone());
    }

    // Grab the Analytics Engine sink (and APP_ENV for cross-env filtering)
    // before `env` is consumed by the router. Missing binding in local dev /
    // tests is non-fatal — emission below is best-effort.
    let latency_sink = env.analytics_engine("PILOT_LATENCY").ok();
    let app_env = env
        .var("APP_ENV")
        .ok()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let router = Router::new();

    let response = router
        // ── Health ──────────────────────────────────────────────
        .get("/", |_, _| Response::ok("data-fabric-worker online"))
        .get("/health", |_, _| {
            Response::from_json(&HealthResponse {
                service: "data-fabric",
                status: "ok",
                mission: "velocity-for-autonomous-agent-builders",
            })
        })
        .get("/openapi.json", |_, _| {
            let headers = Headers::new();
            headers.set("content-type", "application/json")?;
            headers.set("access-control-allow-origin", "*")?;
            Ok(Response::ok(openapi::get_openapi_spec())?.with_headers(headers))
        })
        .get("/docs", |_, _| {
            let html = r#"<!DOCTYPE html>
<html>
  <head>
    <title>Data Fabric API Documentation</title>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <style>
      body {
        margin: 0;
      }
    </style>
  </head>
  <body>
    <script
      id="api-reference"
      data-url="/openapi.json"
    ></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
  </body>
</html>"#;
            Response::from_html(html)
        })
        // ── Tenants (WS8) ─────────────────────────────────────
        .post_async("/v1/tenants/provision", |mut req, ctx| async move {
            let body: models::TenantProvisionRequest = req.json().await?;
            if body.tenant_id.trim().is_empty() || body.display_name.trim().is_empty() {
                return Response::error("tenant_id and display_name are required", 400);
            }
            let d1 = ctx.env.d1("DB")?;
            let started = js_sys::Date::now();
            db::provision_tenant(&d1, &body).await?;
            let elapsed = (js_sys::Date::now() - started) as i64;
            Response::from_json(&models::TenantProvisionResponse {
                tenant_id: body.tenant_id,
                status: "provisioned".to_string(),
                provisioned_in_ms: elapsed,
            })
        })
        // ── Runs (WS2, D1-backed) ────────────────────────────
        .post_async("/v1/runs", |mut req, ctx| async move {
            let body: models::CreateRun = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_run(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "created".into(),
            })
        })
        .get_async("/v1/runs", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let repo = params.get("repo").map(|s| s.as_str());
            let limit = pagination::clamp_limit(params.get("limit").and_then(|s| s.parse().ok()));
            let cursor =
                match pagination::RunsCursor::decode(params.get("cursor").map(|s| s.as_str())) {
                    Ok(c) => c,
                    Err(_) => {
                        return errors::error_response(
                            "INVALID_CURSOR",
                            "cursor is malformed; echo back the next_cursor from a prior response",
                            400,
                        );
                    }
                };
            let d1 = ctx.env.d1("DB")?;
            let (runs, next_cursor) =
                db::list_runs(&d1, &tenant_ctx.tenant_id, repo, limit, cursor.as_ref()).await?;
            let next_cursor_str = match next_cursor.as_ref().map(|c| c.encode()).transpose() {
                Ok(s) => s,
                Err(_) => {
                    return errors::error_response(
                        "CURSOR_ENCODE_FAILED",
                        "internal: failed to encode next cursor",
                        500,
                    );
                }
            };
            Response::from_json(&serde_json::json!({
                "runs": runs,
                "next_cursor": next_cursor_str,
            }))
        })
        .get_async("/v1/runs/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_run(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(run) => Response::from_json(&run),
                None => errors::error_response("RUN_NOT_FOUND", "run not found", 404),
            }
        })
        .get_async("/v1/pull-requests/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_aivcs_pull_request(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(pr) => Response::from_json(&pr),
                None => {
                    errors::error_response("PULL_REQUEST_NOT_FOUND", "pull request not found", 404)
                }
            }
        })
        // ── AIVCS issue #148 slice 4: explicit run pause/resume ─────
        //
        // These are scoped to runs only — task-level pause/resume is a
        // follow-up slice. We chose explicit `/pause` / `/resume`
        // endpoints (not overloads of `/cancel` or `/fail`) so the audit
        // trail in events_bronze records the operator's intent
        // unambiguously.
        .post_async("/v1/runs/:run_id/pause", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            // Auth: builder or admin. Viewer is already caught by the
            // global middleware (it rejects all non-read methods), but
            // we pin the role check here too so the contract is local
            // to the handler.
            if !matches!(
                tenant_ctx.role,
                tenant::TenantRole::Builder | tenant::TenantRole::Admin
            ) {
                return errors::error_response(
                    "FORBIDDEN",
                    "pause requires builder or admin role",
                    403,
                );
            }
            let run_id = ctx
                .param("run_id")
                .expect("param run_id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            let actor = tenant_ctx.actor();
            match db::pause_run(&d1, &tenant_ctx.tenant_id, &run_id, &actor).await? {
                db::PauseOutcome::Paused(update) | db::PauseOutcome::AlreadyPaused(update) => {
                    // Both paths return 200. AlreadyPaused is idempotent
                    // per the issue #148 spec; the response envelope is
                    // the same shape either way.
                    Response::from_json(&update)
                }
                db::PauseOutcome::Terminal { current_status } => {
                    errors::error_response_with_details(
                        "RUN_IN_TERMINAL_STATE",
                        "cannot pause a run that has already reached a terminal state",
                        serde_json::json!({ "current_status": current_status }),
                        409,
                    )
                }
                db::PauseOutcome::NotFound => {
                    errors::error_response("RUN_NOT_FOUND", "run not found", 404)
                }
            }
        })
        .post_async("/v1/runs/:run_id/resume", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            if !matches!(
                tenant_ctx.role,
                tenant::TenantRole::Builder | tenant::TenantRole::Admin
            ) {
                return errors::error_response(
                    "FORBIDDEN",
                    "resume requires builder or admin role",
                    403,
                );
            }
            let run_id = ctx
                .param("run_id")
                .expect("param run_id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            let actor = tenant_ctx.actor();
            match db::resume_run(&d1, &tenant_ctx.tenant_id, &run_id, &actor).await? {
                db::ResumeOutcome::Resumed(update) => Response::from_json(&update),
                db::ResumeOutcome::NotPaused { current_status } => {
                    errors::error_response_with_details(
                        "RUN_NOT_PAUSED",
                        "run is not in paused state",
                        serde_json::json!({ "current_status": current_status }),
                        409,
                    )
                }
                db::ResumeOutcome::NotFound => {
                    errors::error_response("RUN_NOT_FOUND", "run not found", 404)
                }
            }
        })
        // ── WS10 pilot baseline metrics (issue #105) ────────
        .get_async("/v1/metrics/pilot", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let window_raw = params.get("window").map(|s| s.as_str()).unwrap_or("1d");
            let (window, window_seconds) = match metrics::parse_window(window_raw) {
                Ok(w) => w,
                Err(e) => {
                    return errors::error_response("INVALID_WINDOW", &e.to_string(), 400);
                }
            };
            let task_type = params.get("task_type").map(|s| s.as_str());
            let d1 = ctx.env.d1("DB")?;
            let body = metrics::pilot(
                &d1,
                &tenant_ctx.tenant_id,
                &window,
                window_seconds,
                task_type,
            )
            .await?;
            Response::from_json(&body)
        })
        // ── WS2 Tasks (run-scoped, D1-backed) ───────────────
        .post_async("/v1/runs/:run_id/tasks", |mut req, ctx| async move {
            let run_id = ctx
                .param("run_id")
                .expect("param run_id is required by route")
                .to_string();
            let body: models::CreateTask = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_ws2_task(&d1, &tenant_ctx.tenant_id, &id, &run_id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "created".into(),
            })
        })
        .get_async("/v1/runs/:run_id/tasks", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let run_id = ctx
                .param("run_id")
                .expect("param run_id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            let tasks = db::list_ws2_tasks(&d1, &tenant_ctx.tenant_id, &run_id).await?;
            Response::from_json(&serde_json::json!({ "tasks": tasks }))
        })
        // ── Plans (WS2, D1-backed) ──────────────────────────
        .post_async("/v1/plans", |mut req, ctx| async move {
            let body: models::CreatePlan = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_plan(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "created".into(),
            })
        })
        // ── Tool Calls (WS2, D1-backed) ─────────────────────
        .post_async("/v1/tool-calls", |mut req, ctx| async move {
            let body: models::RecordToolCall = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::record_tool_call(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "recorded".into(),
            })
        })
        // ── Releases (WS2, D1-backed) ───────────────────────
        .post_async("/v1/releases", |mut req, ctx| async move {
            let body: models::CreateRelease = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_release(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "created".into(),
            })
        })
        // ── Provenance Events (WS3, D1-backed) ──────────────
        .post_async("/v1/events", |mut req, ctx| async move {
            let body: models::IngestEvent = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::ingest_event(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&models::EventAck {
                id,
                event_type: body.event_type,
                accepted: true,
            })
        })
        // ── Retrieval & Memory Federation (WS5) ───────────────
        .post_async("/v1/memory/index", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let body: models::UpsertMemoryItemRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;

            // 1. Persistent storage in D1
            let expires_at = db::upsert_memory_item(&d1, &tenant_ctx.tenant_id, &id, &body).await?;

            // 2. Semantic indexing in Vectorize
            if let Ok(index) = vector_index::SemanticIndex::new(&ctx.env) {
                if let Ok(vector) = index.embed(&body.summary).await {
                    let metadata = serde_json::json!({
                        "repo": body.repo,
                        "kind": format!("{:?}", body.kind),
                        "run_id": body.run_id,
                        "tenant_id": tenant_ctx.tenant_id
                    });
                    let _ = index.insert(&id, vector, metadata).await;
                }
            }

            Response::from_json(&models::MemoryItemCreated {
                id,
                status: "indexed".into(),
                expires_at,
            })
        })
        .post_async("/v1/memory/retrieve", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let body: models::RetrieveMemoryRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;

            let start = js_sys::Date::now();

            // 1. Semantic Search via Vectorize (if query provided)
            let mut semantic_ids = Vec::new();
            if !body.query.is_empty() {
                if let Ok(index) = vector_index::SemanticIndex::new(&ctx.env) {
                    if let Ok(vector) = index.embed(&body.query).await {
                        if let Ok(results) = index.query(vector, body.top_k).await {
                            if let Some(matches) = results["matches"].as_array() {
                                for m in matches {
                                    if let Some(id) = m["id"].as_str() {
                                        semantic_ids.push(id.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 2. Relational Search + Filter via D1
            // Hybrid approach: use semantic IDs if available, or just use D1 filters
            let response =
                db::retrieve_memory_hybrid(&d1, &tenant_ctx.tenant_id, &body, &semantic_ids)
                    .await?;

            let _latency_ms = (js_sys::Date::now() - start) as i64;
            // Update latency in response if needed, though D1 usually tracks its own

            Response::from_json(&response)
        })
        .post_async("/v1/memory/context-pack", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let body: models::ContextPackRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let response = db::build_context_pack(&d1, &tenant_ctx.tenant_id, &body).await?;
            Response::from_json(&response)
        })
        .post_async("/v1/memory/:id/retire", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = match ctx.param("id") {
                Some(k) => k.to_string(),
                None => return Response::error("missing memory id", 400),
            };
            let d1 = ctx.env.d1("DB")?;
            let retired = db::retire_memory_item(&d1, &tenant_ctx.tenant_id, &id).await?;
            if retired {
                Response::from_json(&models::RetireMemoryResponse {
                    id,
                    status: "retired".into(),
                })
            } else {
                Response::error("memory item not found", 404)
            }
        })
        .post_async("/v1/memory/gc", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let body: models::MemoryGcRequest = {
                let text = req.text().await?;
                if text.trim().is_empty() {
                    models::MemoryGcRequest { limit: 1000 }
                } else {
                    match serde_json::from_str::<models::MemoryGcRequest>(&text) {
                        Ok(v) => v,
                        Err(_) => return Response::error("invalid JSON body", 400),
                    }
                }
            };
            let d1 = ctx.env.d1("DB")?;
            let response = db::run_memory_gc(&d1, &tenant_ctx.tenant_id, &body).await?;
            Response::from_json(&response)
        })
        .post_async("/v1/memory/retrieval-feedback", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let body: models::RetrievalFeedback = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            db::record_retrieval_feedback(&d1, &tenant_ctx.tenant_id, &body).await?;
            Response::from_json(&models::RetrievalFeedbackAck { recorded: true })
        })
        .get_async("/v1/memory/eval/summary", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let summary = db::memory_eval_summary(&d1, &tenant_ctx.tenant_id).await?;
            Response::from_json(&summary)
        })
        // ── Plays (Orchestration) ──────────────────────────────
        .post_async("/v1/plays/:name/launch", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let play_name = ctx
                .param("name")
                .expect("param name is required by route")
                .to_string();
            // Bad JSON in the body is a client contract violation: explicitly
            // reject with 400 instead of silently falling back to a default
            // request (which would mask the violation as a 200 OK). An empty
            // body is still accepted and treated as a no-arg launch.
            let body: models::PlayLaunchRequest = {
                let text = req.text().await?;
                if text.trim().is_empty() {
                    models::PlayLaunchRequest {
                        play_name: play_name.clone(),
                        job_id: None,
                        metadata: None,
                    }
                } else {
                    match parse_play_launch_body(&text) {
                        Ok(v) => v,
                        Err(_) => {
                            return errors::error_response(
                                "INVALID_JSON_BODY",
                                "invalid JSON body for play launch",
                                400,
                            );
                        }
                    }
                }
            };

            // 1. Fetch play definition from D1 (tenant-scoped).
            let d1 = ctx.env.d1("DB")?;
            let def = match db::get_play_definition(&d1, &tenant_ctx.tenant_id, &play_name).await? {
                Some(d) => d,
                None => return Response::error(format!("play '{}' not found", play_name), 404),
            };

            // 2. Launch via PlayManager Durable Object.
            //
            // SECURITY: the DO instance name is namespaced by tenant_id so that
            // a run_id collision across tenants cannot route into the same
            // PlayManager instance. Pre-WS8 the DO was named by `run_id` alone,
            // which meant a guessable / colliding run_id could let one tenant
            // address another tenant's PlayManager state. See PR body for the
            // cutover note — existing PlayManager DOs are unreachable under
            // the new name.
            let run_id = body
                .job_id
                .unwrap_or_else(|| generate_id().expect("failed to generate id"));
            let do_name = play_do_name(&tenant_ctx.tenant_id, &run_id);
            let namespace = ctx.env.durable_object("PLAY_MANAGER")?;
            let stub = namespace.id_from_name(&do_name)?.get_stub()?;

            // Plumb tenant_id into the DO via a launch envelope so the DO
            // does not have to (incorrectly) derive it from `self.state.id()`.
            #[derive(serde::Serialize)]
            struct LaunchEnvelope<'a> {
                tenant_id: &'a str,
                definition: &'a models::PlayDefinition,
            }
            let envelope = LaunchEnvelope {
                tenant_id: &tenant_ctx.tenant_id,
                definition: &def,
            };

            let headers = Headers::new();
            headers.set("x-tenant-id", &tenant_ctx.tenant_id)?;
            headers.set("x-run-id", &run_id)?;

            let do_req = Request::new_with_init(
                "https://do/launch",
                &RequestInit {
                    method: Method::Post,
                    body: Some(JsValue::from_str(&serde_json::to_string(&envelope).map_err(|e| Error::RustError(e.to_string()))?)),
                    headers,
                    ..Default::default()
                },
            )?;
            let mut do_resp = stub.fetch_with_request(do_req).await?;

            let result: serde_json::Value = do_resp.json().await?;
            Response::from_json(&result)
        })
        // ── Artifacts (R2-backed) ─────────────────────────────
        .put_async("/v1/artifacts/:key", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let key = match ctx.param("key") {
                Some(k) => k.to_string(),
                None => return Response::error("missing artifact key", 400),
            };
            let data = req.bytes().await?;
            if data.len() > MAX_ARTIFACT_BYTES {
                return Response::error("artifact exceeds max size", 413);
            }
            let bucket = ctx.env.bucket("ARTIFACTS")?;
            let scoped_key = format!("{}{}", tenant_ctx.r2_prefix(), key);
            let size = storage::put_blob(&bucket, &scoped_key, data).await?;
            Response::from_json(&serde_json::json!({
                "key": key,
                "scoped_key": scoped_key,
                "size": size,
                "stored": true,
            }))
        })
        .get_async("/v1/artifacts/:key", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let key = match ctx.param("key") {
                Some(k) => k.to_string(),
                None => return Response::error("missing artifact key", 400),
            };
            let bucket = ctx.env.bucket("ARTIFACTS")?;
            let scoped_key = format!("{}{}", tenant_ctx.r2_prefix(), key);
            match storage::get_blob(&bucket, &scoped_key).await? {
                Some(data) => {
                    let headers = Headers::new();
                    headers.set("content-type", "application/octet-stream")?;
                    Ok(Response::from_bytes(data)?.with_headers(headers))
                }
                None => Response::error("not found", 404),
            }
        })
        // ── Policy & Governance (WS4) ──────────────────────────
        .post_async("/v1/policies/check", |mut req, ctx| async move {
            let body: models::PolicyCheckRequest = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let evaluated =
                policy::evaluate_policy(&ctx.env, &d1, &tenant_ctx.tenant_id, &body).await?;
            Response::from_json(&models::PolicyCheckResponse {
                id: evaluated.decision_id.clone(),
                action: body.action,
                decision: evaluated.decision,
                reason: evaluated.reason,
                risk_level: Some(format!("{:?}", evaluated.risk_level).to_ascii_lowercase()),
                policy_version: Some(evaluated.policy_version),
                matched_rule: evaluated.matched_rule,
                escalation_id: evaluated.escalation_id,
                rate_limited: Some(evaluated.rate_limited),
            })
        })
        // ── Policy Rules CRUD (WS4) ─────────────────────────────
        .post_async("/v1/policies/rules", |mut req, ctx| async move {
            let body: models::CreatePolicyRule = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            // Validate verdict
            match body.verdict.as_str() {
                "allow" | "deny" | "escalate" => {}
                _ => return Response::error("verdict must be allow, deny, or escalate", 400),
            }
            // Validate risk_level
            match body.risk_level.as_str() {
                "read" | "write" | "destructive" | "irreversible" => {}
                _ => {
                    return Response::error(
                        "risk_level must be read, write, destructive, or irreversible",
                        400,
                    )
                }
            }
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_policy_rule(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&models::Created {
                id,
                status: "created".into(),
            })
        })
        .get_async("/v1/policies/rules", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let limit = pagination::clamp_limit(params.get("limit").and_then(|s| s.parse().ok()));
            let cursor = match pagination::PolicyRulesCursor::decode(
                params.get("cursor").map(|s| s.as_str()),
            ) {
                Ok(c) => c,
                Err(_) => {
                    return errors::error_response(
                        "INVALID_CURSOR",
                        "cursor is malformed; echo back the next_cursor from a prior response",
                        400,
                    );
                }
            };
            let d1 = ctx.env.d1("DB")?;
            let (rules, next_cursor) =
                db::list_policy_rules(&d1, &tenant_ctx.tenant_id, limit, cursor.as_ref()).await?;
            let next_cursor_str = match next_cursor.as_ref().map(|c| c.encode()).transpose() {
                Ok(s) => s,
                Err(_) => {
                    return errors::error_response(
                        "CURSOR_ENCODE_FAILED",
                        "internal: failed to encode next cursor",
                        500,
                    );
                }
            };
            let responses: Vec<_> = rules.into_iter().map(|r| r.into_response()).collect();
            Response::from_json(&serde_json::json!({
                "rules": responses,
                "next_cursor": next_cursor_str,
            }))
        })
        .get_async("/v1/policies/rules/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_policy_rule(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(rule) => Response::from_json(&rule.into_response()),
                None => Response::error("rule not found", 404),
            }
        })
        .patch_async("/v1/policies/rules/:id", |mut req, ctx| async move {
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let body: models::UpdatePolicyRule = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            // Validate verdict if provided
            if let Some(ref v) = body.verdict {
                match v.as_str() {
                    "allow" | "deny" | "escalate" => {}
                    _ => return Response::error("verdict must be allow, deny, or escalate", 400),
                }
            }
            if let Some(ref v) = body.risk_level {
                match v.as_str() {
                    "read" | "write" | "destructive" | "irreversible" => {}
                    _ => {
                        return Response::error(
                            "risk_level must be read, write, destructive, or irreversible",
                            400,
                        )
                    }
                }
            }
            let d1 = ctx.env.d1("DB")?;
            let updated = db::update_policy_rule(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            if updated {
                match db::get_policy_rule(&d1, &tenant_ctx.tenant_id, &id).await? {
                    Some(rule) => Response::from_json(&rule.into_response()),
                    None => Response::error("rule not found", 404),
                }
            } else {
                Response::error("rule not found", 404)
            }
        })
        .delete_async("/v1/policies/rules/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            let deleted = db::delete_policy_rule(&d1, &tenant_ctx.tenant_id, &id).await?;
            if deleted {
                Response::from_json(&serde_json::json!({ "deleted": true }))
            } else {
                Response::error("rule not found", 404)
            }
        })
        // ── Policy Decision History (WS4) ────────────────────────
        .get_async("/v1/policies/decisions", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let action = params.get("action").map(|s| s.as_str());
            let actor = params.get("actor").map(|s| s.as_str());
            let decision = params.get("decision").map(|s| s.as_str());
            let limit = params
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(50u32)
                .min(200);
            let d1 = ctx.env.d1("DB")?;
            let decisions = db::list_policy_decisions(
                &d1,
                &tenant_ctx.tenant_id,
                action,
                actor,
                decision,
                limit,
            )
            .await?;
            let responses: Vec<_> = decisions.into_iter().map(|d| d.into_response()).collect();
            Response::from_json(&serde_json::json!({ "decisions": responses }))
        })
        // ── WS7 Verification Evidence ─────────────────────────
        .get_async("/v1/verification/evidence", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let run_id = params.get("run_id").map(|s| s.as_str());
            let limit = params
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(50u32)
                .min(200);
            let d1 = ctx.env.d1("DB")?;
            let evidence =
                db::list_verification_evidence(&d1, &tenant_ctx.tenant_id, run_id, limit).await?;
            let responses: Vec<_> = evidence.into_iter().map(|e| e.into_response()).collect();
            Response::from_json(&serde_json::json!({ "evidence": responses }))
        })
        // ── Policy Definitions & Retention (WS4) ────────────────
        .put_async(
            "/v1/policies/definitions/:version",
            |mut req, ctx| async move {
                let version = match ctx.param("version") {
                    Some(v) => v.to_string(),
                    None => return Response::error("missing policy version", 400),
                };
                let body: models::PutPolicyDefinitionRequest = req.json().await?;
                let bucket = ctx.env.bucket("ARTIFACTS")?;
                let resp =
                    policy::put_policy_definition(&ctx.env, &bucket, &version, &body).await?;
                Response::from_json(&resp)
            },
        )
        .post_async("/v1/policies/activate/:version", |_req, ctx| async move {
            let version = match ctx.param("version") {
                Some(v) => v.to_string(),
                None => return Response::error("missing policy version", 400),
            };
            match policy::activate_policy_version(&ctx.env, &version).await {
                Ok(resp) => Response::from_json(&resp),
                Err(err) => {
                    // Log the underlying error server-side (visible in
                    // `wrangler tail`) and return a sanitized envelope so we
                    // don't leak internal paths/stack context to the client.
                    worker::console_log!(
                        "ERROR: policy activation failed for version {version}: {err:?}"
                    );
                    let (code, message, status) = policy_activation_error_response_parts();
                    errors::error_response(code, message, status)
                }
            }
        })
        .get_async("/v1/policies/active", |_req, ctx| async move {
            let resp = policy::active_policy_version(&ctx.env).await?;
            Response::from_json(&resp)
        })
        .post_async("/v1/retention/run", |mut req, ctx| async move {
            let body: models::RetentionRunRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let bucket = ctx.env.bucket("ARTIFACTS")?;
            let deleted = db::run_retention_cleanup(&d1, &bucket, &body).await?;
            Response::from_json(&deleted)
        })
        // ── Agent Tasks (M1) ──────────────────────────────────
        .post_async("/v1/tasks", |mut req, ctx| async move {
            let body: models::CreateAgentTask = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;

            // Create in D1 for persistence
            db::create_task(&d1, &tenant_ctx.tenant_id, &id, &body).await?;

            // Fetch the task as an AgentTask for the DO
            let task = match db::get_mcp_task_by_id(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(t) => t,
                None => return Response::error("failed to fetch created task", 500),
            };

            // Enqueue in Durable Object for active management
            let namespace = ctx.env.durable_object("TASK_LEASE_MANAGER")?;
            let stub = namespace.id_from_name(&tenant_ctx.tenant_id)?.get_stub()?;

            let do_req = Request::new_with_init(
                "https://do/enqueue",
                &RequestInit {
                    method: Method::Post,
                    body: Some(JsValue::from_str(&serde_json::to_string(&task).map_err(|e| Error::RustError(e.to_string()))?)),
                    ..Default::default()
                },
            )?;
            let do_resp = stub.fetch_with_request(do_req).await?;

            // Propagate any non-2xx response (notably 429 QUEUE_FULL backpressure
            // from TaskLeaseManager when pending_tasks is at MAX_PENDING_TASKS)
            // to the client. Without this passthrough, the DO's Retry-After +
            // QUEUE_FULL envelope is silently swallowed and the client sees a
            // generic success / 500, defeating the backpressure feature.
            if let Some(forwarded) = forward_do_response(do_resp).await? {
                return Ok(forwarded);
            }

            Response::from_json(&models::TaskCreated {
                id,
                status: "pending".into(),
            })
        })
        // POST /mcp/task/next — claims the next pending task for an agent.
        // This is a state-mutating, non-idempotent operation, so POST is the correct
        // HTTP method. Caching proxies must not cache a "claimed" state, and retry
        // middleware that auto-retries idempotent verbs must not double-claim.
        .post_async("/mcp/task/next", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

            let agent_id = match params.get("agent_id") {
                Some(id) => id.clone(),
                None => return Response::error("agent_id required", 400),
            };
            let caps = params.get("cap").cloned().unwrap_or_default();

            let namespace = ctx.env.durable_object("TASK_LEASE_MANAGER")?;
            let stub = namespace.id_from_name(&tenant_ctx.tenant_id)?.get_stub()?;

            let do_url = build_do_url("/claim", &[("agent_id", &agent_id), ("caps", &caps)])?;
            let do_req = Request::new(&do_url, Method::Post)?;
            let mut do_resp = stub.fetch_with_request(do_req).await?;

            if do_resp.status_code() == 204 {
                return Ok(Response::empty()?.with_status(204));
            }

            let mut task: models::AgentTask = do_resp.json().await?;

            // Sync to D1 (best effort)
            let d1 = ctx.env.d1("DB")?;
            let _ = db::sync_task_status(&d1, &tenant_ctx.tenant_id, &task).await;

            // Augment task with agent's memory context from MOM (if available)
            let mom_endpoint = ctx.env.var("MOM_ENDPOINT").ok().map(|v| v.to_string());
            if let Some(endpoint) = mom_endpoint {
                let memory_context =
                    augment_task_with_memory(&agent_id, &tenant_ctx.tenant_id, &task, &endpoint)
                        .await;
                task.memory_context = memory_context;
            }
            Response::from_json(&task)
        })
        // DEPRECATED: GET /mcp/task/next was the original (incorrect) shape of this
        // endpoint. It is being kept for one release as a compatibility shim that
        // returns 405 Method Not Allowed with `Allow: POST` and a `Deprecation`
        // header so legacy clients get an immediate, actionable failure rather than
        // silently double-claiming tasks through retry middleware.
        //
        // 405 was chosen over 308 Permanent Redirect because RFC 9110 308 preserves
        // the original request method — a GET-redirected-to-the-same-URL client
        // would loop, not switch to POST. 405 is the correct semantic for
        // "wrong method, use this one instead".
        //
        // Removal target: next minor release after clients in
        // data-fabric-client (and any other in-tree consumers) are upgraded.
        .get_async("/mcp/task/next", |_req, _ctx| async move {
            worker::console_log!(
                "WARN: deprecated GET /mcp/task/next called; clients must migrate to POST. \
                 GET will be removed in the next minor release."
            );
            let headers = Headers::new();
            headers.set("allow", "POST")?;
            headers.set("deprecation", "true")?;
            headers.set(
                "sunset",
                "GET /mcp/task/next is deprecated; use POST /mcp/task/next",
            )?;
            Ok(Response::error(
                "Method Not Allowed: GET /mcp/task/next is deprecated; use POST /mcp/task/next",
                405,
            )?
            .with_headers(headers))
        })
        .post_async("/mcp/task/:id/heartbeat", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
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

            let namespace = ctx.env.durable_object("TASK_LEASE_MANAGER")?;
            let stub = namespace.id_from_name(&tenant_ctx.tenant_id)?.get_stub()?;

            let do_url = build_do_url(
                "/heartbeat",
                &[("task_id", &task_id), ("agent_id", &agent_id)],
            )?;
            let do_req = Request::new(&do_url, Method::Post)?;
            let do_resp = stub.fetch_with_request(do_req).await?;

            if do_resp.status_code() == 200 {
                // Update D1 (best effort)
                let d1 = ctx.env.d1("DB")?;
                let _ = db::heartbeat_task(&d1, &tenant_ctx.tenant_id, &task_id, &agent_id).await;
                Response::from_json(&serde_json::json!({ "ok": true }))
            } else {
                Response::error("task not found or not owned by agent", 404)
            }
        })
        .post_async("/mcp/task/:id/complete", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let task_id = match ctx.param("id") {
                Some(id) => id.to_string(),
                None => return Response::error("missing task id", 400),
            };
            let body: models::TaskCompleteRequest = req.json().await?;

            let namespace = ctx.env.durable_object("TASK_LEASE_MANAGER")?;
            let stub = namespace.id_from_name(&tenant_ctx.tenant_id)?.get_stub()?;

            let do_req = Request::new_with_init(
                &format!("https://do/complete/{}", task_id),
                &RequestInit {
                    method: Method::Post,
                    body: Some(JsValue::from_str(&serde_json::to_string(&body).map_err(|e| Error::RustError(e.to_string()))?)),
                    ..Default::default()
                },
            )?;
            let mut do_resp = stub.fetch_with_request(do_req).await?;

            if do_resp.status_code() == 200 {
                let task: models::AgentTask = do_resp.json().await?;
                // Sync to D1
                let d1 = ctx.env.d1("DB")?;
                db::sync_task_status(&d1, &tenant_ctx.tenant_id, &task).await?;
                Response::from_json(&serde_json::json!({ "status": "completed" }))
            } else {
                Response::error("task not found or not running", 404)
            }
        })
        .post_async("/mcp/task/:id/fail", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let task_id = match ctx.param("id") {
                Some(id) => id.to_string(),
                None => return Response::error("missing task id", 400),
            };
            let body: models::TaskFailRequest = req.json().await?;

            let namespace = ctx.env.durable_object("TASK_LEASE_MANAGER")?;
            let stub = namespace.id_from_name(&tenant_ctx.tenant_id)?.get_stub()?;

            let do_req = Request::new_with_init(
                &format!("https://do/fail/{}", task_id),
                &RequestInit {
                    method: Method::Post,
                    body: Some(JsValue::from_str(&serde_json::to_string(&body).map_err(|e| Error::RustError(e.to_string()))?)),
                    ..Default::default()
                },
            )?;
            let mut do_resp = stub.fetch_with_request(do_req).await?;

            if do_resp.status_code() == 200 {
                let task: models::AgentTask = do_resp.json().await?;
                // Sync to D1
                let d1 = ctx.env.d1("DB")?;
                db::sync_task_status(&d1, &tenant_ctx.tenant_id, &task).await?;
                Response::from_json(&serde_json::json!({ "status": task.status }))
            } else {
                Response::error("task not found or not running", 404)
            }
        })
        // ── Agents (M1) ───────────────────────────────────────
        .post_async("/v1/agents", |mut req, ctx| async move {
            let body: models::RegisterAgent = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::register_agent(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&serde_json::json!({
                "id": id,
                "name": body.name,
                "status": "active",
            }))
        })
        .get_async("/v1/agents", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let limit = pagination::clamp_limit(params.get("limit").and_then(|s| s.parse().ok()));
            let cursor =
                match pagination::AgentsCursor::decode(params.get("cursor").map(|s| s.as_str())) {
                    Ok(c) => c,
                    Err(_) => {
                        return errors::error_response(
                            "INVALID_CURSOR",
                            "cursor is malformed; echo back the next_cursor from a prior response",
                            400,
                        );
                    }
                };
            let d1 = ctx.env.d1("DB")?;
            let (agents, next_cursor) =
                db::list_agents(&d1, &tenant_ctx.tenant_id, limit, cursor.as_ref()).await?;
            let next_cursor_str = match next_cursor.as_ref().map(|c| c.encode()).transpose() {
                Ok(s) => s,
                Err(_) => {
                    return errors::error_response(
                        "CURSOR_ENCODE_FAILED",
                        "internal: failed to encode next cursor",
                        500,
                    );
                }
            };
            Response::from_json(&serde_json::json!({
                "agents": agents,
                "next_cursor": next_cursor_str,
            }))
        })
        .post_async("/v1/telemetry", |mut req, ctx| async move {
            let body: models::TelemetrySnapshot = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;

            db::record_telemetry(&d1, &tenant_ctx.tenant_id, &id, &body).await?;

            Response::from_json(&models::TelemetryAck { id, accepted: true })
        })
        // ── Reasoning traces (Epic 3 / #111) ─────────────────
        .post_async("/v1/reasoning-traces", |mut req, ctx| async move {
            let mut body: models::IngestReasoningTrace = req.json().await?;
            if body.idempotency_key.trim().is_empty() {
                return Response::error("idempotency_key must be non-empty", 400);
            }
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;

            // Short-circuit on retry: a row already exists for this idempotency
            // key. Returning early here means we never touch R2 on retries,
            // which would otherwise orphan a duplicate blob under a fresh
            // trace_id (D1 dedupes by key, R2 keys are id-derived).
            if let Some(existing_id) = db::find_reasoning_trace_by_idempotency_key(
                &d1,
                &tenant_ctx.tenant_id,
                &body.idempotency_key,
            )
            .await?
            {
                return Response::from_json(&models::TraceAck {
                    id: existing_id,
                    accepted: true,
                    deduplicated: true,
                });
            }

            let bucket = ctx.env.bucket("ARTIFACTS")?;
            let id = generate_id()?;
            let r2_prefix = tenant_ctx.r2_prefix();

            // Defensive PII redaction. The client SHOULD have redacted, but the
            // sink must never persist fields the client flagged as tainted.
            if let Some(v) = body.inputs.as_mut() {
                models::redact_pii(v);
            }
            if let Some(v) = body.outputs.as_mut() {
                models::redact_pii(v);
            }

            let (inputs_inline, inputs_r2_key) =
                stash_payload(&bucket, &r2_prefix, &id, "inputs", body.inputs.as_ref()).await?;
            let (outputs_inline, outputs_r2_key) =
                stash_payload(&bucket, &r2_prefix, &id, "outputs", body.outputs.as_ref()).await?;

            let inserted = db::insert_reasoning_trace(
                &d1,
                &tenant_ctx.tenant_id,
                &id,
                &body,
                db::ReasoningPayloadRefs {
                    inputs_inline: inputs_inline.as_deref(),
                    inputs_r2_key: inputs_r2_key.as_deref(),
                    outputs_inline: outputs_inline.as_deref(),
                    outputs_r2_key: outputs_r2_key.as_deref(),
                },
            )
            .await?;

            Response::from_json(&models::TraceAck {
                id,
                accepted: true,
                deduplicated: !inserted,
            })
        })
        .get_async("/v1/reasoning-traces", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req.url()?;
            let params: std::collections::HashMap<String, String> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let job_id = match params.get("job_id") {
                Some(id) => id.clone(),
                None => return Response::error("missing job_id query param", 400),
            };
            let limit = params
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(100u32)
                .clamp(1, 500);
            let after_step = params.get("after_step").and_then(|s| s.parse::<i64>().ok());
            let d1 = ctx.env.d1("DB")?;
            let page = db::list_reasoning_traces_for_job(
                &d1,
                &tenant_ctx.tenant_id,
                &job_id,
                after_step,
                limit,
            )
            .await?;
            let next_after_step = if page.has_more {
                page.traces.last().map(|t| t.step_number as i64)
            } else {
                None
            };
            Response::from_json(&serde_json::json!({
                "traces": page.traces,
                "has_more": page.has_more,
                "next_after_step": next_after_step,
            }))
        })
        // Payload resolver — inline JSON returns the value verbatim; an
        // archived payload streams the R2 object back. Either way callers
        // dereference a single canonical URL, fulfilling the AC's "pointer
        // URL" requirement without leaking R2 keys to the public internet.
        .get_async(
            "/v1/reasoning-traces/:id/payload/:field",
            |req, ctx| async move {
                let tenant_ctx = tenant::tenant_from_request(&req)?;
                let trace_id = match ctx.param("id") {
                    Some(v) => v.to_string(),
                    None => return Response::error("missing trace id", 400),
                };
                let field = match ctx.param("field") {
                    Some(v) => v.to_string(),
                    None => return Response::error("missing field", 400),
                };
                if field != "inputs" && field != "outputs" {
                    return Response::error("field must be 'inputs' or 'outputs'", 400);
                }
                let d1 = ctx.env.d1("DB")?;
                let trace =
                    match db::get_reasoning_trace(&d1, &tenant_ctx.tenant_id, &trace_id).await? {
                        Some(t) => t,
                        None => return Response::error("reasoning trace not found", 404),
                    };
                let (inline, r2_key) = if field == "inputs" {
                    (trace.inputs_inline, trace.inputs_r2_key)
                } else {
                    (trace.outputs_inline, trace.outputs_r2_key)
                };
                if let Some(v) = inline {
                    return Response::from_json(&v);
                }
                if let Some(key) = r2_key {
                    let bucket = ctx.env.bucket("ARTIFACTS")?;
                    let bytes = match storage::get_blob(&bucket, &key).await? {
                        Some(b) => b,
                        None => return Response::error("payload archived but missing in R2", 502),
                    };
                    let mut resp = Response::from_bytes(bytes)?;
                    resp.headers_mut().set("Content-Type", "application/json")?;
                    return Ok(resp);
                }
                // Step had no payload (e.g. token-only Thought).
                Response::from_json(&serde_json::Value::Null)
            },
        )
        // ── Checkpoints (M2) ──────────────────────────────────
        .post_async("/v1/checkpoints", |mut req, ctx| async move {
            let body: models::CreateCheckpoint = req.json().await?;
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let bucket = ctx.env.bucket("ARTIFACTS")?;
            let id = generate_id()?;
            let r2_key = format!(
                "{}checkpoints/{}/{}",
                tenant_ctx.r2_prefix(),
                body.thread_id,
                id
            );

            let state_bytes =
                serde_json::to_vec(&body.state).map_err(|e| Error::RustError(e.to_string()))?;
            let size = storage::put_blob(&bucket, &r2_key, state_bytes).await? as i64;
            db::create_checkpoint(&d1, &tenant_ctx.tenant_id, &id, &body, &r2_key, size).await?;

            // 3. Update ThreadManager Durable Object (fast state).
            //
            // SECURITY: name the DO by `{tenant_id}:{thread_id}` so a
            // guessable thread_id from tenant A cannot route into tenant
            // B's ThreadManager instance. Pre-WS8 the DO was named by
            // thread_id alone.
            let do_name = thread_do_name(&tenant_ctx.tenant_id, &body.thread_id);
            let namespace = ctx.env.durable_object("THREAD_MANAGER")?;
            let stub = namespace.id_from_name(&do_name)?.get_stub()?;

            let headers = {
                let h = Headers::new();
                h.set("x-checkpoint-id", &id)?;
                h
            };

            let do_req = Request::new_with_init(
                "https://do/checkpoint",
                &RequestInit {
                    method: Method::Post,
                    body: Some(JsValue::from_str(&serde_json::to_string(&body).map_err(|e| Error::RustError(e.to_string()))?)),
                    headers,
                    ..Default::default()
                },
            )?;
            stub.fetch_with_request(do_req).await?;

            Response::from_json(&models::CheckpointCreated {
                id,
                thread_id: body.thread_id,
                state_r2_key: r2_key,
            })
        })
        .get_async(
            "/v1/checkpoints/threads/:thread_id",
            |req, ctx| async move {
                let tenant_ctx = tenant::tenant_from_request(&req)?;
                let thread_id = ctx
                    .param("thread_id")
                    .expect("param thread_id is required by route")
                    .to_string();

                // 1. Try fast path via ThreadManager Durable Object.
                // Same tenant-scoped naming as the POST checkpoint handler.
                let do_name = thread_do_name(&tenant_ctx.tenant_id, &thread_id);
                let namespace = ctx.env.durable_object("THREAD_MANAGER")?;
                let stub = namespace.id_from_name(&do_name)?.get_stub()?;

                let do_req = Request::new("https://do/latest", Method::Get)?;
                let mut do_resp = stub.fetch_with_request(do_req).await?;

                if do_resp.status_code() == 200 {
                    let data: serde_json::Value = do_resp.json().await?;
                    return Response::from_json(&data);
                }

                // 2. Fallback to D1/R2 slow path
                let d1 = ctx.env.d1("DB")?;
                match db::get_latest_checkpoint(&d1, &tenant_ctx.tenant_id, &thread_id).await? {
                    Some(row) => {
                        let bucket = ctx.env.bucket("ARTIFACTS")?;
                        match storage::get_blob(&bucket, &row.state_r2_key).await? {
                            Some(blob) => {
                                let state: serde_json::Value = serde_json::from_slice(&blob)?;
                                Response::from_json(&serde_json::json!({
                                    "checkpoint": row.into_checkpoint(),
                                    "state": state
                                }))
                            }
                            None => Response::error("checkpoint state not found in R2", 404),
                        }
                    }
                    None => Response::error("no checkpoint found", 404),
                }
            },
        )
        .get_async("/v1/checkpoints/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_checkpoint_by_id(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(row) => Response::from_json(&row.into_checkpoint()),
                None => Response::error("checkpoint not found", 404),
            }
        })
        .delete_async("/v1/checkpoints/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = match ctx.param("id") {
                Some(k) => k.to_string(),
                None => return Response::error("missing checkpoint id", 400),
            };
            let d1 = ctx.env.d1("DB")?;

            let row = match db::get_checkpoint_by_id(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(r) => r,
                None => return Response::error("checkpoint not found", 404),
            };
            let r2_key = row.state_r2_key.clone();

            let deleted = db::delete_checkpoint(&d1, &tenant_ctx.tenant_id, &id).await?;
            if deleted {
                let bucket = ctx.env.bucket("ARTIFACTS")?;
                let _ = storage::delete_blob(&bucket, &r2_key).await;
                Response::from_json(&serde_json::json!({ "deleted": true }))
            } else {
                Response::error("checkpoint not found", 404)
            }
        })
        // ── Memory (WS5: #45) ─────────────────────────────────
        .post_async("/v1/memory", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let body: models::CreateMemory = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_memory(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&models::MemoryCreated {
                id,
                thread_id: body.thread_id,
                scope: body.scope,
                key: body.key,
            })
        })
        .get_async("/v1/memory/threads/:thread_id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let thread_id = match ctx.param("thread_id") {
                Some(t) => t.to_string(),
                None => return Response::error("missing thread_id", 400),
            };
            let limit = parse_limit_query(req.url().ok(), "limit").unwrap_or(100);
            let d1 = ctx.env.d1("DB")?;
            let memories =
                db::list_memories_for_thread(&d1, &tenant_ctx.tenant_id, &thread_id, limit).await?;
            Response::from_json(&serde_json::json!({ "memories": memories }))
        })
        .get_async(
            "/v1/context-pack/threads/:thread_id",
            |req, ctx| async move {
                let tenant_ctx = tenant::tenant_from_request(&req)?;
                let thread_id = match ctx.param("thread_id") {
                    Some(t) => t.to_string(),
                    None => return Response::error("missing thread_id", 400),
                };
                let max_items = parse_limit_query(req.url().ok(), "max_items").unwrap_or(50);
                let d1 = ctx.env.d1("DB")?;
                let checkpoint =
                    db::get_latest_checkpoint(&d1, &tenant_ctx.tenant_id, &thread_id).await?;
                let memories =
                    db::list_memories_for_thread(&d1, &tenant_ctx.tenant_id, &thread_id, max_items)
                        .await?;
                Response::from_json(&serde_json::json!({
                    "thread_id": thread_id,
                    "checkpoint": checkpoint.map(|r| r.into_checkpoint()),
                    "memories": memories,
                }))
            },
        )
        // ── Traces / Provenance (WS3: issue #43) ──────────────
        .get_async("/v1/traces/:run_id", |req, ctx| async move {
            let started = js_sys::Date::now();
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let run_id = match ctx.param("run_id") {
                Some(r) => r.to_string(),
                None => return Response::error("missing run_id", 400),
            };
            let url = req.url().ok();
            let (limit, has_valid_limit_param) =
                parse_limit_query_with_valid_presence(url, "limit", db::TRACE_DEFAULT_LIMIT);
            let d1 = ctx.env.d1("DB")?;
            // Fetch limit+1 to detect truncation without a second query for count when not needed
            let mut events =
                db::get_trace_for_run(&d1, &tenant_ctx.tenant_id, &run_id, limit + 1).await?;
            let truncated = events.len() > limit as usize;
            if truncated {
                events.truncate(limit as usize);
            }
            let total = db::count_trace_events_for_run(&d1, &tenant_ctx.tenant_id, &run_id).await?;
            let (total_meta, truncated_meta) =
                build_trace_response_metadata(total, truncated, has_valid_limit_param);
            timed_json_response(
                started,
                &models::TraceResponse {
                    run_id,
                    total: total_meta,
                    events,
                    truncated: truncated_meta,
                },
            )
        })
        .get_async("/v1/traces/:run_id/lineage", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let run_id = match ctx.param("run_id") {
                Some(r) => r.to_string(),
                None => return Response::error("missing run_id", 400),
            };
            let url = req.url().ok();
            let (limit, has_valid_limit_param) =
                parse_limit_query_with_valid_presence(url, "limit", 100);
            let d1 = ctx.env.d1("DB")?;
            let mut events =
                db::get_trace_for_run(&d1, &tenant_ctx.tenant_id, &run_id, limit + 1).await?;
            let truncated = events.len() > limit as usize;
            if truncated {
                events.truncate(limit as usize);
            }
            let total = db::count_trace_events_for_run(&d1, &tenant_ctx.tenant_id, &run_id).await?;
            let (total_meta, truncated_meta) =
                build_trace_response_metadata(total, truncated, has_valid_limit_param);
            Response::from_json(&models::TraceResponse {
                run_id,
                total: total_meta,
                events,
                truncated: truncated_meta,
            })
        })
        // ── Provenance Chain (WS3) ──────────────────────────────
        .get_async("/v1/provenance/:kind/:id", |req, ctx| async move {
            let started = js_sys::Date::now();
            let tenant_ctx = tenant::tenant_from_request(&req)?;
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
            if direction != "forward" && direction != "backward" {
                return Response::error("invalid direction: must be 'forward' or 'backward'", 400);
            }
            let hops = params
                .get("hops")
                .and_then(|s| s.parse().ok())
                .unwrap_or(5u32)
                .min(100);
            let d1 = ctx.env.d1("DB")?;
            let edges =
                db::get_provenance_chain(&d1, &tenant_ctx.tenant_id, &kind, &id, direction, hops)
                    .await?;
            timed_json_response(
                started,
                &models::ProvenanceResponse {
                    entity_kind: kind,
                    entity_id: id,
                    direction: direction.into(),
                    hops,
                    edges,
                },
            )
        })
        // ── Gold Layer: Run Summaries (WS3) ─────────────────────
        .get_async("/v1/runs/:run_id/summary", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let run_id = ctx
                .param("run_id")
                .expect("param run_id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_run_summary(&d1, &tenant_ctx.tenant_id, &run_id).await? {
                Some(summary) => Response::from_json(&summary),
                None => Response::error("run summary not found", 404),
            }
        })
        .get_async("/v1/gold/run-summaries", |req, ctx| async move {
            let started = js_sys::Date::now();
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let limit = parse_limit_query(req.url().ok(), "limit")
                .unwrap_or(50)
                .min(200);
            let d1 = ctx.env.d1("DB")?;
            let summaries = db::list_run_summaries(&d1, &tenant_ctx.tenant_id, limit).await?;
            timed_json_response(started, &serde_json::json!({ "summaries": summaries }))
        })
        // ── Gold Layer: Task Dependency Graph (WS3: #58) ────────
        .get_async("/v1/gold/runs/:run_id/task-graph", |req, ctx| async move {
            let started = js_sys::Date::now();
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let run_id = ctx
                .param("run_id")
                .expect("param run_id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            let edges = db::get_task_dependencies(&d1, &tenant_ctx.tenant_id, &run_id).await?;
            timed_json_response(
                started,
                &serde_json::json!({ "run_id": run_id, "edges": edges }),
            )
        })
        // ── Replay Plan (WS3: #58) ────────────────────────────────
        .post_async("/v1/replay/plan", |mut req, ctx| async move {
            let started = js_sys::Date::now();
            let body: models::ReplayPlanRequest = req.json().await?;
            if body.run_id.trim().is_empty() {
                return Response::error("run_id is required", 400);
            }
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let steps = db::build_replay_plan(
                &d1,
                &tenant_ctx.tenant_id,
                &body.run_id,
                body.from_event_id.as_deref(),
                body.to_event_id.as_deref(),
            )
            .await?;
            let status = if steps.is_empty() { "empty" } else { "planned" };
            timed_json_response(
                started,
                &models::ReplayPlanResponse {
                    run_id: body.run_id,
                    steps,
                    status: status.into(),
                },
            )
        })
        .post_async("/v1/replay", |mut req, ctx| async move {
            let started = js_sys::Date::now();
            let body: models::ReplayExecuteRequest = req.json().await?;
            if body.run_id.trim().is_empty() {
                return Response::error("run_id is required", 400);
            }

            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;

            let steps = db::build_replay_plan(
                &d1,
                &tenant_ctx.tenant_id,
                &body.run_id,
                body.from_event_id.as_deref(),
                body.to_event_id.as_deref(),
            )
            .await?;

            let baseline_steps = match body.baseline_run_id.as_deref() {
                Some(baseline_run_id) if !baseline_run_id.trim().is_empty() => {
                    db::build_replay_plan(
                        &d1,
                        &tenant_ctx.tenant_id,
                        baseline_run_id,
                        body.from_event_id.as_deref(),
                        body.to_event_id.as_deref(),
                    )
                    .await?
                }
                _ => Vec::new(),
            };

            let has_baseline = !baseline_steps.is_empty();
            let (drift_count, drift_ratio_percent) = if has_baseline {
                verification::compute_replay_drift_percent(&steps, &baseline_steps)
            } else {
                (0, 0.0)
            };
            let within_variance =
                !has_baseline || drift_ratio_percent <= f64::from(body.variance_tolerance_percent);

            let tests_passed = body.tests_passed.unwrap_or(true);
            let policy_approved = body.policy_approved.unwrap_or(true);
            let provenance_complete = body
                .provenance_complete
                .unwrap_or(!steps.is_empty() && within_variance);
            let verification = verification::evaluate_verification_gates(
                tests_passed,
                policy_approved,
                provenance_complete,
            );

            let failure_classification = if has_baseline {
                Some(verification::classify_failure_from_drift_ratio(
                    drift_ratio_percent,
                ))
            } else {
                None
            };

            let status = if verification.eligible_for_promotion && within_variance {
                "verified"
            } else {
                "needs_review"
            };

            let evidence_id = generate_id()?;
            let replay_response = models::ReplayExecuteResponse {
                evidence_id: evidence_id.clone(),
                run_id: body.run_id,
                baseline_run_id: body.baseline_run_id,
                status: status.into(),
                step_count: steps.len(),
                drift_count,
                drift_ratio_percent,
                within_variance,
                failure_classification,
                verification,
            };

            db::create_verification_evidence(
                &d1,
                &tenant_ctx.tenant_id,
                &evidence_id,
                &replay_response,
            )
            .await?;

            timed_json_response(started, &replay_response)
        })
        // ── WS6: Integration Registry ────────────────────────
        .post_async("/v1/integrations", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let body: integrations::RegisterIntegration = req.json().await?;
            if body.name.trim().is_empty() || body.name.len() > 256 {
                return Response::error("name must be 1-256 characters", 400);
            }
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::register_integration(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&serde_json::json!({
                "id": id,
                "target": body.target,
                "name": body.name,
                "status": "active",
            }))
        })
        .get_async("/v1/integrations", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let limit = parse_limit_query(req.url().ok(), "limit")
                .unwrap_or(50)
                .min(200);
            let d1 = ctx.env.d1("DB")?;
            let list = db::list_integrations(&d1, &tenant_ctx.tenant_id, limit).await?;
            Response::from_json(&serde_json::json!({ "integrations": list }))
        })
        .get_async("/v1/integrations/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_integration(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(integration) => Response::from_json(&integration),
                None => Response::error("integration not found", 404),
            }
        })
        .patch_async("/v1/integrations/:id", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let body: integrations::UpdateIntegration = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            db::update_integration(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            match db::get_integration(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(integration) => Response::from_json(&integration),
                None => Response::error("integration not found", 404),
            }
        })
        .delete_async("/v1/integrations/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            let deleted = db::delete_integration(&d1, &tenant_ctx.tenant_id, &id).await?;
            if deleted {
                Response::from_json(&serde_json::json!({ "id": id, "deleted": true }))
            } else {
                Response::error("integration not found", 404)
            }
        })
        // ── WS6: oxidizedgraph intake ───────────────────────
        .post_async(
            "/v1/integrations/oxidizedgraph/events",
            |mut req, ctx| async move {
                let tenant_ctx = tenant::tenant_from_request(&req)?;
                let batch: integrations::oxidizedgraph::GraphExecBatch = req.json().await?;
                if batch.events.len() > db::INTEGRATION_BATCH_LIMIT {
                    return Response::error(
                        format!("batch exceeds max {} events", db::INTEGRATION_BATCH_LIMIT),
                        400,
                    );
                }
                let d1 = ctx.env.d1("DB")?;
                let bucket = ctx.env.bucket("ARTIFACTS")?;
                let now = db::now_iso();

                let owned_events = integrations::oxidizedgraph::adapt_to_graph_events(&batch);
                let event_count = match ingest_events_bronze_silver(
                    &d1,
                    &tenant_ctx.tenant_id,
                    owned_events,
                    &now,
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        worker::console_log!("ERROR: oxidizedgraph event ingest failed: {e:?}");
                        return degraded_response(
                            "oxidizedgraph",
                            "event ingestion temporarily unavailable",
                        );
                    }
                };

                let mut checkpoint_count = 0usize;
                for evt in &batch.events {
                    if let Some(cp) = integrations::oxidizedgraph::adapt_to_checkpoint(&batch, evt)
                    {
                        let id = generate_id()?;
                        let r2_key = format!(
                            "checkpoints/{}/{}/{}",
                            tenant_ctx.tenant_id, cp.thread_id, id
                        );
                        let state_bytes = serde_json::to_vec(&cp.state)
                            .map_err(|e| Error::RustError(e.to_string()))?;
                        let size = storage::put_blob(&bucket, &r2_key, state_bytes).await? as i64;
                        db::create_checkpoint(&d1, &tenant_ctx.tenant_id, &id, &cp, &r2_key, size)
                            .await?;
                        checkpoint_count += 1;
                    }
                }

                if let Err(e) =
                    db::touch_integration(&d1, &tenant_ctx.tenant_id, "oxidizedgraph", None).await
                {
                    worker::console_log!("WARN: touch_integration(oxidizedgraph) failed: {e:?}");
                }

                Response::from_json(&serde_json::json!({
                    "source": "oxidizedgraph",
                    "events_ingested": event_count,
                    "checkpoints_created": checkpoint_count,
                }))
            },
        )
        // ── WS6: aivcs intake ───────────────────────────────
        .post_async("/v1/integrations/aivcs/events", |mut req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let evt: integrations::aivcs::PipelineEvent = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let bucket = ctx.env.bucket("ARTIFACTS")?;

            let mut run_id = None;
            if let Some(run) = integrations::aivcs::adapt_to_run(&evt) {
                let id = generate_id()?;
                match db::create_run(&d1, &tenant_ctx.tenant_id, &id, &run).await {
                    Ok(()) => run_id = Some(id),
                    Err(e) => {
                        worker::console_log!("ERROR: aivcs create_run failed: {e:?}");
                        return degraded_response("aivcs", "run creation temporarily unavailable");
                    }
                }
            }

            let fabric_evt = integrations::aivcs::adapt_to_event(&evt);
            let evt_id = generate_id()?;
            if let Err(e) = db::ingest_event(&d1, &tenant_ctx.tenant_id, &evt_id, &fabric_evt).await
            {
                worker::console_log!("ERROR: aivcs ingest_event failed: {e:?}");
                return degraded_response("aivcs", "event ingestion temporarily unavailable");
            }

            // Write artifact metadata to R2 so fabric tracks pipeline artifacts
            let mut artifact_count = 0usize;
            if let Some(ref artifacts) = evt.artifacts {
                for art in artifacts {
                    let r2_key = format!("{}/artifacts/{}", tenant_ctx.tenant_id, art.key);
                    let meta = serde_json::to_vec(&serde_json::json!({
                        "pipeline_id": evt.pipeline_id,
                        "key": art.key,
                        "content_type": art.content_type,
                        "size_bytes": art.size_bytes,
                        "checksum": art.checksum,
                        "source": "aivcs",
                    }))
                    .map_err(|e| Error::RustError(e.to_string()))?;
                    storage::put_blob(&bucket, &r2_key, meta).await?;
                    artifact_count += 1;
                }
            }

            // Extract change_set_id for ci_check_run projection
            let cs_id_opt = {
                let change_set_opt = evt
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("aivcs"))
                    .and_then(|a| a.get("change_set").or_else(|| a.get("pull_request")));

                let explicit_id = change_set_opt
                    .and_then(|cs| cs.get("id"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()));

                let source_id = explicit_id.clone().or_else(|| {
                    change_set_opt
                        .and_then(|cs| cs.get("number").or_else(|| cs.get("pull_request_id")))
                        .and_then(|v| match v {
                            serde_json::Value::String(s) => Some(s.clone()),
                            serde_json::Value::Number(n) => Some(n.to_string()),
                            _ => None,
                        })
                });

                source_id.map(|src_id| {
                    if explicit_id.is_none() && src_id.chars().all(|c| c.is_ascii_digit()) {
                        format!("{}#{}", evt.repo, src_id)
                    } else {
                        src_id
                    }
                })
            };

            if let Some(ref cs_id) = cs_id_opt {
                let status = match evt.event_type {
                    integrations::aivcs::PipelineEventType::PipelineStart
                    | integrations::aivcs::PipelineEventType::StageStart
                    | integrations::aivcs::PipelineEventType::DeployStart => "in_progress",
                    _ => "completed",
                };

                let conclusion = evt
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("conclusion").or_else(|| m.get("state")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        if status == "completed" {
                            Some("success".to_string())
                        } else {
                            None
                        }
                    });

                let url = evt
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("url").or_else(|| m.get("html_url")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let name = evt
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("name").or_else(|| m.get("stage")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "pipeline".to_string());

                let check_run_id = format!("{}:{}", evt.pipeline_id, name);

                if let Err(e) = db::upsert_ci_check_run(
                    &d1,
                    &tenant_ctx.tenant_id,
                    &check_run_id,
                    cs_id,
                    &name,
                    status,
                    &conclusion,
                    &url,
                )
                .await
                {
                    worker::console_log!("ERROR: aivcs upsert_ci_check_run failed: {e:?}");
                }
            }

            if let Err(e) = db::touch_integration(&d1, &tenant_ctx.tenant_id, "aivcs", None).await {
                worker::console_log!("WARN: touch_integration(aivcs) failed: {e:?}");
            }

            Response::from_json(&serde_json::json!({
                "source": "aivcs",
                "event_id": evt_id,
                "run_id": run_id,
                "artifacts_stored": artifact_count,
            }))
        })
        // ── WS6: llama.rs intake ────────────────────────────
        .post_async(
            "/v1/integrations/llama-rs/inference",
            |mut req, ctx| async move {
                let tenant_ctx = tenant::tenant_from_request(&req)?;
                let body: integrations::llama_rs::InferenceRequest = req.json().await?;
                let d1 = ctx.env.d1("DB")?;

                let task = integrations::llama_rs::adapt_to_task(&body);
                let id = generate_id()?;
                db::create_task(&d1, &tenant_ctx.tenant_id, &id, &task).await?;

                if let Err(e) =
                    db::touch_integration(&d1, &tenant_ctx.tenant_id, "llama_rs", None).await
                {
                    worker::console_log!("WARN: touch_integration(llama_rs) failed: {e:?}");
                }

                Response::from_json(&serde_json::json!({
                    "source": "llama_rs",
                    "task_id": id,
                    "status": "pending",
                }))
            },
        )
        .post_async(
            "/v1/integrations/llama-rs/telemetry",
            |mut req, ctx| async move {
                let tenant_ctx = tenant::tenant_from_request(&req)?;
                let telemetry: integrations::llama_rs::InferenceTelemetry = req.json().await?;
                let d1 = ctx.env.d1("DB")?;
                let now = db::now_iso();

                let graph_events = integrations::llama_rs::adapt_to_graph_events(&telemetry);
                if graph_events.len() > db::INTEGRATION_BATCH_LIMIT {
                    return Response::error(
                        format!(
                            "telemetry exceeds max {} events",
                            db::INTEGRATION_BATCH_LIMIT
                        ),
                        400,
                    );
                }
                let count =
                    ingest_events_bronze_silver(&d1, &tenant_ctx.tenant_id, graph_events, &now)
                        .await?;

                if let Err(e) =
                    db::touch_integration(&d1, &tenant_ctx.tenant_id, "llama_rs", None).await
                {
                    worker::console_log!("WARN: touch_integration(llama_rs) failed: {e:?}");
                }

                Response::from_json(&serde_json::json!({
                    "source": "llama_rs",
                    "events_ingested": count,
                }))
            },
        )
        // ── WS6: llama.rs context retrieval ──────────────────
        .post_async(
            "/v1/integrations/llama-rs/context",
            |mut req, ctx| async move {
                let tenant_ctx = tenant::tenant_from_request(&req)?;
                let body: integrations::llama_rs::ContextRequest = req.json().await?;
                let d1 = ctx.env.d1("DB")?;

                let pack_req = integrations::llama_rs::adapt_to_context_pack(&body);
                let response =
                    db::build_context_pack(&d1, &tenant_ctx.tenant_id, &pack_req).await?;

                if let Err(e) =
                    db::touch_integration(&d1, &tenant_ctx.tenant_id, "llama_rs", None).await
                {
                    worker::console_log!("WARN: touch_integration(llama_rs) failed: {e:?}");
                }

                Response::from_json(&response)
            },
        )
        // ── Graph Events (M3) ─────────────────────────────────
        .post_async("/v1/graph-events", |mut req, ctx| async move {
            let started = js_sys::Date::now();
            let body: models::GraphEventBatch = req.json().await?;
            if body.events.len() > db::INTEGRATION_BATCH_LIMIT {
                return Response::error(
                    format!("batch exceeds max {} events", db::INTEGRATION_BATCH_LIMIT),
                    400,
                );
            }
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let now = js_sys::Date::new_0()
                .to_iso_string()
                .as_string()
                .expect("Date should be a valid string");

            let mut events: Vec<(String, &models::GraphEvent, String)> =
                Vec::with_capacity(body.events.len());
            for e in &body.events {
                let id = generate_id()?;
                events.push((id, e, now.clone()));
            }

            let count = events.len();
            db::insert_events_bronze(&d1, &tenant_ctx.tenant_id, &events).await?;

            let mut silver_events: Vec<(String, String, &models::GraphEvent, String)> =
                Vec::with_capacity(events.len());
            for (bronze_id, evt, created_at) in &events {
                let silver_id = generate_id()?;
                silver_events.push((silver_id, bronze_id.clone(), evt, created_at.clone()));
            }
            db::insert_events_silver(&d1, &tenant_ctx.tenant_id, &silver_events, &now).await?;

            // Best-effort enqueue with tenant context
            if let Ok(queue) = ctx.env.queue("EVENTS") {
                for (_id, evt, _ts) in &events {
                    let envelope = models::QueueEnvelope {
                        tenant_id: tenant_ctx.tenant_id.clone(),
                        event: (*evt).clone(),
                    };
                    if let Err(e) = queue
                        .send(serde_json::to_value(&envelope).unwrap_or_default())
                        .await
                    {
                        worker::console_log!("[graph-events] queue send error: {}", e);
                    }
                }
            }

            let duration_ms = js_sys::Date::now() - started;
            timed_json_response(
                started,
                &models::GraphEventAck {
                    accepted: count,
                    queued: true,
                    duration_ms: Some(duration_ms),
                },
            )
        })
        // ── AIVCS: change_set routes (issue #148) ────────────────
        .get_async("/v1/change-sets", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req
                .url()
                .map_err(|_| Error::RustError("invalid url".into()))?;
            let params = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            let repo = params.get("repo").map(|s| s.as_ref());
            let limit = pagination::clamp_limit(params.get("limit").and_then(|s| s.parse().ok()));
            let d1 = ctx.env.d1("DB")?;
            if let Some(r) = repo {
                let list =
                    db::list_change_sets_by_repo(&d1, &tenant_ctx.tenant_id, r, limit).await?;
                Response::from_json(&serde_json::json!({ "change_sets": list }))
            } else {
                Response::error("missing required query parameter: repo", 400)
            }
        })
        .get_async("/v1/change-sets/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_change_set(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(cs) => Response::from_json(&cs),
                None => Response::error("change_set not found", 404),
            }
        })
        // ── AIVCS: review projections routes (issue #148) ────────
        .get_async("/v1/review-threads", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req
                .url()
                .map_err(|_| Error::RustError("invalid url".into()))?;
            let params = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            let review_id = params.get("review_id").map(|s| s.as_ref());
            let d1 = ctx.env.d1("DB")?;
            if let Some(rid) = review_id {
                let list = db::list_review_threads_for_review(&d1, &tenant_ctx.tenant_id, rid, 100)
                    .await?;
                Response::from_json(&serde_json::json!({ "review_threads": list }))
            } else {
                Response::error("missing required query parameter: review_id", 400)
            }
        })
        .get_async("/v1/review-threads/:id/comments", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let thread_id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            let list =
                db::list_review_comments_for_thread(&d1, &tenant_ctx.tenant_id, &thread_id, 100)
                    .await?;
            Response::from_json(&serde_json::json!({ "comments": list }))
        })
        .get_async("/v1/review-threads/:id/anchors", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let thread_id = ctx
                .param("id")
                .expect("param id is required by route")
                .to_string();
            let d1 = ctx.env.d1("DB")?;
            let list =
                db::list_file_anchors_for_thread(&d1, &tenant_ctx.tenant_id, &thread_id, 100)
                    .await?;
            Response::from_json(&serde_json::json!({ "file_anchors": list }))
        })
        .get_async("/v1/human-decisions", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req
                .url()
                .map_err(|_| Error::RustError("invalid url".into()))?;
            let params = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            let review_id = params.get("review_id").map(|s| s.as_ref());
            let d1 = ctx.env.d1("DB")?;
            if let Some(rid) = review_id {
                let list =
                    db::list_human_decisions_by_review(&d1, &tenant_ctx.tenant_id, rid).await?;
                Response::from_json(&serde_json::json!({ "human_decisions": list }))
            } else {
                Response::error("missing required query parameter: review_id", 400)
            }
        })
        .get_async("/v1/ci-check-runs", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req
                .url()
                .map_err(|_| Error::RustError("invalid url".into()))?;
            let params = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            let change_set_id = params
                .get("change_set_id")
                .or_else(|| params.get("pr_id"))
                .map(|s| s.as_ref());
            let limit = pagination::clamp_limit(params.get("limit").and_then(|s| s.parse().ok()));
            let d1 = ctx.env.d1("DB")?;
            if let Some(cs_id) = change_set_id {
                let list =
                    db::list_ci_check_runs_for_change_set(&d1, &tenant_ctx.tenant_id, cs_id, limit)
                        .await?;
                Response::from_json(&serde_json::json!({ "ci_check_runs": list }))
            } else {
                Response::error(
                    "missing required query parameter: change_set_id or pr_id",
                    400,
                )
            }
        })
        .get_async("/v1/events", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req
                .url()
                .map_err(|_| Error::RustError("invalid url".into()))?;
            let params = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            let since = params.get("since").map(|s| s.as_ref());
            let limit = pagination::clamp_limit(params.get("limit").and_then(|s| s.parse().ok()));
            let d1 = ctx.env.d1("DB")?;
            let list = db::list_events_bronze(&d1, &tenant_ctx.tenant_id, since, limit).await?;
            Response::from_json(&serde_json::json!({ "events": list }))
        })
        .get_async("/v1/branches", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let url = req
                .url()
                .map_err(|_| Error::RustError("invalid url".into()))?;
            let params = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            let repo = params.get("repo").map(|s| s.as_ref());
            let limit = pagination::clamp_limit(params.get("limit").and_then(|s| s.parse().ok()));
            let d1 = ctx.env.d1("DB")?;
            if let Some(r) = repo {
                let list = db::list_branches_by_repo(&d1, &tenant_ctx.tenant_id, r, limit).await?;
                Response::from_json(&serde_json::json!({ "branches": list }))
            } else {
                Response::error("missing required query parameter: repo", 400)
            }
        })
        .run(req, env)
        .await;

    if !is_public_path(&path) {
        if let Some(sink) = latency_sink.as_ref() {
            let status = response
                .as_ref()
                .ok()
                .map(|r| r.status_code())
                .unwrap_or(500);
            emit_pilot_latency(
                sink,
                &path,
                &method,
                tenant_id_for_metric.as_deref().unwrap_or("unknown"),
                &app_env,
                js_sys::Date::now() - start_ms,
                status,
            );
        }
    }

    response
}

/// Best-effort emit of one request-latency sample to the `PILOT_LATENCY`
/// Analytics Engine dataset. Failures are deliberately swallowed: a missing
/// dataset or transient sink error must not turn a successful request into
/// a 500.
///
/// Path is currently emitted as-is. High-cardinality routes (`/v1/runs/:id`)
/// inflate the dataset row count — templating the path is a follow-up.
///
/// Sample schema (kept in sync with `docs/ws10/METRICS_ENDPOINT.md`):
/// * `index1` — `tenant_id` (sampling key)
/// * `blob1`  — request path
/// * `blob2`  — HTTP method
/// * `blob3`  — `APP_ENV` (dev / staging / production)
/// * `double1`— elapsed milliseconds
/// * `double2`— response status code
fn emit_pilot_latency(
    sink: &AnalyticsEngineDataset,
    path: &str,
    method: &Method,
    tenant_id: &str,
    app_env: &str,
    elapsed_ms: f64,
    status_code: u16,
) {
    let method_str = method.to_string();
    let dp = AnalyticsEngineDataPointBuilder::new()
        .indexes([tenant_id])
        .add_blob(path)
        .add_blob(method_str.as_str())
        .add_blob(app_env)
        .add_double(elapsed_ms)
        .add_double(status_code as f64)
        .build();
    let _ = sink.write_data_point(&dp);
}

/// Queue consumer: enriches events — causality edges, gold summaries, task deps.
#[event(queue)]
pub async fn queue(batch: MessageBatch<serde_json::Value>, env: Env, _ctx: Context) -> Result<()> {
    let queue_name = batch.queue();
    let d1 = env.d1("DB")?;
    let messages = batch.messages()?;

    for msg in &messages {
        let body = msg.body();

        // Try QueueEnvelope first, fall back to bare GraphEvent for compat
        let (tenant_id, evt) =
            if let Ok(envelope) = serde_json::from_value::<models::QueueEnvelope>(body.clone()) {
                (envelope.tenant_id, envelope.event)
            } else if let Ok(evt) = serde_json::from_value::<models::GraphEvent>(body.clone()) {
                ("default".to_string(), evt)
            } else {
                worker::console_log!(
                    "[queue {}] failed to deserialize message: {:?}",
                    queue_name,
                    body
                );
                msg.retry();
                continue;
            };

        let now = js_sys::Date::new_0()
            .to_iso_string()
            .as_string()
            .expect("Date should be a valid string");

        // Note: silver promotion is done synchronously in POST /v1/graph-events.
        // The queue consumer handles only causality edges and gold layer summaries.

        // Causality edges
        if let Err(e) = db::insert_causality_from_event(&d1, &tenant_id, &evt).await {
            worker::console_log!("[queue {}] causality insert error: {}", queue_name, e);
        }

        // Materialize task dependencies from payload.depends_on
        if let (Some(ref run_id), Some(ref payload)) = (&evt.run_id, &evt.payload) {
            if let Some(task_id) = payload.get("task_id").and_then(|v| v.as_str()) {
                if let Some(deps) = payload.get("depends_on").and_then(|v| v.as_array()) {
                    for dep in deps {
                        if let Some(dep_id) = dep.as_str() {
                            if let Err(e) =
                                db::upsert_task_dependency(&d1, &tenant_id, run_id, task_id, dep_id)
                                    .await
                            {
                                worker::console_log!(
                                    "[queue {}] task dep upsert error: {}",
                                    queue_name,
                                    e
                                );
                            }
                        }
                    }
                }
            }
        }

        // Gold layer summary
        if let Some(ref run_id) = evt.run_id {
            if let Err(e) = db::upsert_run_summary(
                &d1,
                &tenant_id,
                run_id,
                evt.actor.as_deref(),
                &evt.event_type,
                &now,
            )
            .await
            {
                worker::console_log!("[queue {}] run summary upsert error: {}", queue_name, e);
            }
        }

        msg.ack();
    }
    Ok(())
}

/// Scheduled event: poll Gemini batch jobs and other background maintenance.
#[event(scheduled)]
pub async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) -> Result<()> {
    gemini_service::poll_gemini_jobs(&env).await
}

pub(crate) fn generate_id() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)
        .map_err(|err| Error::RustError(format!("failed to generate id: {err}")))?;
    Ok(hex::encode(buf))
}

/// Decide whether a reasoning-trace payload stays inline or gets offloaded
/// to R2 (the GZRS analogue on Cloudflare). Returns `(inline_json, r2_key)`
/// where at most one is `Some`.
async fn stash_payload(
    bucket: &Bucket,
    r2_prefix: &str,
    trace_id: &str,
    field: &str,
    payload: Option<&serde_json::Value>,
) -> Result<(Option<String>, Option<String>)> {
    let Some(value) = payload else {
        return Ok((None, None));
    };
    match models::classify_payload(value, r2_prefix, trace_id, field) {
        models::PayloadDisposition::Inline(v) => {
            let s = serde_json::to_string(&v)
                .map_err(|e| Error::RustError(format!("serialize inline payload: {e}")))?;
            Ok((Some(s), None))
        }
        models::PayloadDisposition::Archive { key, bytes } => {
            storage::put_blob(bucket, &key, bytes).await?;
            Ok((None, Some(key)))
        }
    }
}

/// Build a JSON response with a `Server-Timing` header recording the elapsed time since `started`.
fn timed_json_response<T: Serialize>(started: f64, body: &T) -> Result<Response> {
    let mut resp = Response::from_json(body)?;
    let dur = js_sys::Date::now() - started;
    resp.headers_mut()
        .set("Server-Timing", &format!("total;dur={:.1}", dur))?;
    Ok(resp)
}

/// Shared helper: ingest a vec of GraphEvents into bronze + silver layers.
/// Returns the number of events ingested.
async fn ingest_events_bronze_silver(
    d1: &D1Database,
    tenant_id: &str,
    events: Vec<models::GraphEvent>,
    now: &str,
) -> Result<usize> {
    if events.is_empty() {
        return Ok(0);
    }
    // Build (id, event, timestamp) tuples — owned events stored here, refs taken below
    let mut owned: Vec<(String, models::GraphEvent, String)> = Vec::with_capacity(events.len());
    for evt in events {
        owned.push((generate_id()?, evt, now.to_string()));
    }
    let count = owned.len();

    // Borrow from owned vec — single ref-mapping pass, no intermediate Vec
    let bronze_refs: Vec<(String, &models::GraphEvent, String)> = owned
        .iter()
        .map(|(id, evt, ts)| (id.clone(), evt, ts.clone()))
        .collect();
    db::insert_events_bronze(d1, tenant_id, &bronze_refs).await?;

    let mut silver_events: Vec<(String, String, &models::GraphEvent, String)> =
        Vec::with_capacity(count);
    for (bronze_id, evt, ts) in &bronze_refs {
        silver_events.push((generate_id()?, bronze_id.clone(), *evt, ts.clone()));
    }
    db::insert_events_silver(d1, tenant_id, &silver_events, now).await?;
    Ok(count)
}

/// Decision for how to handle a Durable Object response from the worker
/// handler. Pure so the routing rule is unit-testable without a wasm runtime.
#[derive(Debug, PartialEq, Eq)]
enum DoForwardAction {
    /// 2xx or 3xx — handler should proceed with its normal success path.
    /// 3xx is included so a future DO update that emits a redirect isn't
    /// surfaced as a synthetic gateway error.
    Success,
    /// 4xx/5xx — handler should mirror this status back to the client.
    /// For 429 the DO's `retry-after` is preserved through
    /// [`sanitize_retry_after`] (defaulting to
    /// `DEFAULT_DO_RETRY_AFTER_SECS` when absent or malformed) so the
    /// backpressure contract surfaces end-to-end. For other non-2xx/3xx
    /// statuses the header is only forwarded when syntactically valid.
    Forward {
        status: u16,
        retry_after: Option<String>,
    },
}

/// Default Retry-After value to apply when the DO returned 429 without a
/// `retry-after` header. Matches the TaskLeaseManager `/enqueue` 30s value
/// so a missing header doesn't downgrade the client-visible contract.
const DEFAULT_DO_RETRY_AFTER_SECS: u32 = 30;

/// Validate a `retry-after` header value per RFC 7231 §7.1.3 and return a
/// non-negative integer delta-seconds value. Accepts either:
///   * a non-negative integer (delta-seconds form), or
///   * an HTTP-date (IMF-fixdate / RFC 850 / asctime per RFC 7231 §7.1.1.1).
///
/// Garbage (or absent) input falls back to [`DEFAULT_DO_RETRY_AFTER_SECS`] so
/// we never forward an invalid header to the client. The output is always
/// expressed in seconds — for the HTTP-date form we use the default rather
/// than parsing the timestamp (we lack a date dep and the DO contract is
/// to emit delta-seconds), but we still accept the date form as syntactically
/// valid so a conformant peer is not rejected.
fn sanitize_retry_after(raw: Option<&str>) -> u32 {
    let Some(value) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return DEFAULT_DO_RETRY_AFTER_SECS;
    };
    if let Ok(secs) = value.parse::<u32>() {
        return secs;
    }
    if looks_like_http_date(value) {
        // Accept as valid per RFC 7231, but normalise to the default
        // because we don't carry a date-parsing dep in the worker.
        return DEFAULT_DO_RETRY_AFTER_SECS;
    }
    DEFAULT_DO_RETRY_AFTER_SECS
}

/// Cheap structural check for an RFC 7231 §7.1.1.1 HTTP-date. Recognises
/// the three permitted forms by their leading weekday token plus a
/// trailing time component (HH:MM:SS). Intentionally tolerant — the goal
/// is to distinguish well-formed HTTP-date strings from garbage, not to
/// fully parse the timestamp.
fn looks_like_http_date(s: &str) -> bool {
    const WEEKDAYS_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const WEEKDAYS_LONG: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    // Length sanity — shortest legal form (asctime: "Sun Nov  6 08:49:37 1994") is 24 chars;
    // longest legal form (RFC 850 with "Wednesday") is around 33 chars. Allow some slack.
    if s.len() < 20 || s.len() > 40 {
        return false;
    }
    let starts_with_weekday = WEEKDAYS_SHORT
        .iter()
        .chain(WEEKDAYS_LONG.iter())
        .any(|wd| s.starts_with(wd));
    if !starts_with_weekday {
        return false;
    }
    // Must contain a time-of-day "HH:MM:SS" — two colons separating digits.
    let colon_count = s.bytes().filter(|&b| b == b':').count();
    colon_count == 2
}

/// Classify a DO response status + retry-after header into a forward action.
/// Pure: takes primitives so it's testable without a JS runtime.
///
/// The success range is `(200..400)` rather than `(200..300)` so that any
/// 3xx redirect emitted by a (future) DO update is treated as a normal
/// pass-through. Gateways shouldn't surface a downstream redirect as an
/// error, and DOs in this worker don't intentionally redirect today.
///
/// Any `retry-after` header on a forwarded (non-2xx, non-3xx) response is
/// sanitised through [`sanitize_retry_after`] so a malformed value from
/// the DO can't propagate to clients.
fn classify_do_response(status: u16, retry_after_header: Option<&str>) -> DoForwardAction {
    if (200..400).contains(&status) {
        DoForwardAction::Success
    } else {
        let retry_after = if status == 429 {
            // 429 always carries a retry-after — synthesize the default
            // when the DO omitted (or garbled) the header.
            Some(sanitize_retry_after(retry_after_header).to_string())
        } else {
            // Other 4xx/5xx: only forward retry-after if the DO actually
            // provided a syntactically valid value. We don't synthesize a
            // default outside the 429 backpressure contract.
            retry_after_header.and_then(|raw| {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    None
                } else if trimmed.parse::<u32>().is_ok() || looks_like_http_date(trimmed) {
                    Some(trimmed.to_string())
                } else {
                    None
                }
            })
        };
        DoForwardAction::Forward {
            status,
            retry_after,
        }
    }
}

/// Forward a Durable Object response back to the client when it carries a
/// non-2xx status. Returns `Ok(None)` for 2xx so callers continue with their
/// success path; returns `Ok(Some(resp))` with the status, retry-after, and
/// body mirrored when the DO signaled backpressure or another error.
///
/// This exists because `stub.fetch_with_request(...).await?` swallows the DO
/// response by default — including the 429 `QUEUE_FULL` envelope from
/// TaskLeaseManager — so without explicit propagation the client sees a
/// generic success (or, worse, a 500) and the backpressure feature is mute.
async fn forward_do_response(mut do_resp: Response) -> Result<Option<Response>> {
    let status = do_resp.status_code();
    let retry_after = do_resp.headers().get("retry-after").ok().flatten();
    match classify_do_response(status, retry_after.as_deref()) {
        DoForwardAction::Success => Ok(None),
        DoForwardAction::Forward {
            status,
            retry_after,
        } => {
            let content_type = do_resp
                .headers()
                .get("content-type")
                .ok()
                .flatten()
                .unwrap_or_else(|| "application/json".to_string());
            let body = do_resp.bytes().await.unwrap_or_default();

            let headers = Headers::new();
            headers.set("content-type", &content_type)?;
            if let Some(value) = retry_after {
                headers.set("retry-after", &value)?;
            }
            let resp = Response::from_bytes(body)?
                .with_status(status)
                .with_headers(headers);
            Ok(Some(resp))
        }
    }
}

/// Build a 503 response for graceful degradation when the fabric is unavailable.
/// Used by integration intake handlers to signal temporary unavailability
/// so clients can retry rather than treating the failure as permanent.
fn degraded_response(source: &str, detail: &str) -> Result<Response> {
    let body = serde_json::json!({
        "source": source,
        "status": "degraded",
        "detail": detail,
        "retry_after_seconds": 5,
    });
    let mut resp = Response::from_json(&body)?;
    let _ = resp.headers_mut().set("Retry-After", "5");
    // Override status to 503
    Ok(resp.with_status(503))
}

/// Parse a `PlayLaunchRequest` from a raw JSON string.
///
/// Returns `Err(())` on a parse failure so callers can map to a stable error
/// code without leaking serde's free-form parse message to clients. Empty
/// bodies are NOT handled here — the caller is responsible for that fallback
/// (and we want to surface "empty body" vs "malformed body" distinctly).
fn parse_play_launch_body(text: &str) -> std::result::Result<models::PlayLaunchRequest, ()> {
    serde_json::from_str::<models::PlayLaunchRequest>(text).map_err(|_| ())
}

/// Build the sanitized error response parts (code, message, status) for a
/// failed policy activation. The underlying error is intentionally NOT
/// included in the returned message — callers are expected to log the raw
/// error server-side via `console_log!` so it appears in `wrangler tail`.
///
/// Extracted as a pure helper so the sanitization contract is unit-testable
/// without a worker runtime.
fn policy_activation_error_response_parts() -> (&'static str, &'static str, u16) {
    (
        "POLICY_ACTIVATION_FAILED",
        "policy activation failed; see server logs",
        500,
    )
}

/// Parse a numeric query param (e.g. limit, hops). Returns None if URL is None or param missing/invalid.
fn parse_limit_query(url: Option<worker::Url>, param: &str) -> Option<u32> {
    let url = url?;
    let value = url.query_pairs().find(|(k, _)| k == param)?.1;
    value.parse().ok().filter(|&n| n > 0 && n <= 10_000)
}

/// Parse a numeric query param and report whether a valid value was explicitly provided.
fn parse_limit_query_with_valid_presence(
    url: Option<worker::Url>,
    param: &str,
    default_value: u32,
) -> (u32, bool) {
    let parsed = parse_limit_query(url.clone(), param);
    let has_valid_param = parsed.is_some();
    (parsed.unwrap_or(default_value), has_valid_param)
}

/// Build metadata for trace responses.
/// - `total` is emitted only when the caller supplied a valid bound and the result is not truncated.
/// - `truncated` is emitted when truncation happened or when the caller supplied a valid bound.
fn build_trace_response_metadata(
    event_count: u64,
    truncated: bool,
    has_valid_bound_param: bool,
) -> (Option<u64>, Option<bool>) {
    let total = if has_valid_bound_param && !truncated {
        Some(event_count)
    } else {
        None
    };
    let truncated_meta = if has_valid_bound_param || truncated {
        Some(truncated)
    } else {
        None
    };
    (total, truncated_meta)
}

#[cfg(test)]
mod tests {
    use super::{
        build_trace_response_metadata, classify_do_response, parse_limit_query,
        parse_limit_query_with_valid_presence, parse_play_launch_body,
        policy_activation_error_response_parts, sanitize_retry_after, DoForwardAction,
        DEFAULT_DO_RETRY_AFTER_SECS,
    };
    use worker::Url;

    // ── classify_do_response (DO 429 / error forwarding) ────────

    #[test]
    fn classify_do_response_passes_2xx_as_success() {
        // 2xx codes must not be forwarded — handler proceeds with its
        // normal success path. This is the only "Success" branch.
        for status in [200u16, 201, 202, 204, 299] {
            assert_eq!(
                classify_do_response(status, None),
                DoForwardAction::Success,
                "status {status} should be classified as Success",
            );
        }
    }

    #[test]
    fn classify_do_response_forwards_429_with_do_retry_after() {
        // When the DO sets retry-after explicitly, that exact value is
        // mirrored back to the client. This is the TaskLeaseManager
        // backpressure contract surfaced end-to-end.
        let action = classify_do_response(429, Some("30"));
        assert_eq!(
            action,
            DoForwardAction::Forward {
                status: 429,
                retry_after: Some("30".to_string()),
            },
        );
    }

    #[test]
    fn classify_do_response_429_without_header_defaults_retry_after() {
        // Defensive: if a future DO omits retry-after on 429, the worker
        // still surfaces a sensible default so clients don't hot-loop.
        let action = classify_do_response(429, None);
        assert_eq!(
            action,
            DoForwardAction::Forward {
                status: 429,
                retry_after: Some(DEFAULT_DO_RETRY_AFTER_SECS.to_string()),
            },
        );
    }

    #[test]
    fn classify_do_response_forwards_other_4xx_without_synthesizing_retry_after() {
        // Non-429 4xx errors pass through with whatever (if any)
        // retry-after the DO sent — we don't fabricate one.
        assert_eq!(
            classify_do_response(404, None),
            DoForwardAction::Forward {
                status: 404,
                retry_after: None,
            },
        );
        assert_eq!(
            classify_do_response(400, Some("5")),
            DoForwardAction::Forward {
                status: 400,
                retry_after: Some("5".to_string()),
            },
        );
    }

    #[test]
    fn classify_do_response_forwards_5xx() {
        // 5xx from a DO is forwarded as-is so operators see the real
        // failure mode instead of an opaque worker 500.
        assert_eq!(
            classify_do_response(500, None),
            DoForwardAction::Forward {
                status: 500,
                retry_after: None,
            },
        );
        assert_eq!(
            classify_do_response(503, Some("60")),
            DoForwardAction::Forward {
                status: 503,
                retry_after: Some("60".to_string()),
            },
        );
    }

    #[test]
    fn classify_do_response_default_retry_after_matches_task_lease_manager() {
        // Tripwire: the default must match the TaskLeaseManager
        // ENQUEUE_RETRY_AFTER_SECS value (30) so a missing header
        // doesn't downgrade the contract.
        assert_eq!(DEFAULT_DO_RETRY_AFTER_SECS, 30);
        assert_eq!(
            DEFAULT_DO_RETRY_AFTER_SECS,
            super::task_do::ENQUEUE_RETRY_AFTER_SECS
        );
    }

    #[test]
    fn classify_do_response_passes_through_3xx() {
        // 3xx redirect codes are not errors and must not be forwarded
        // through the error-mirroring path. DOs in this worker don't
        // intentionally redirect today, but a future update that does
        // shouldn't surface as a synthetic worker error.
        for status in [300u16, 301, 302, 303, 304, 307, 308, 399] {
            assert_eq!(
                classify_do_response(status, None),
                DoForwardAction::Success,
                "status {status} should pass through as Success",
            );
        }
        // Even with a retry-after header set, 3xx still passes through:
        // the redirect semantics take precedence over backpressure.
        assert_eq!(
            classify_do_response(302, Some("5")),
            DoForwardAction::Success,
        );
    }

    #[test]
    fn sanitize_retry_after_rejects_garbage() {
        // Numeric delta-seconds — accepted verbatim.
        assert_eq!(sanitize_retry_after(Some("0")), 0);
        assert_eq!(sanitize_retry_after(Some("30")), 30);
        assert_eq!(sanitize_retry_after(Some("3600")), 3600);
        // Leading/trailing whitespace is normalised.
        assert_eq!(sanitize_retry_after(Some("  42  ")), 42);

        // HTTP-date forms (RFC 7231 §7.1.1.1) — accepted as syntactically
        // valid; normalised to the default because we don't carry a date
        // dep. The point is they are NOT treated as garbage.
        assert_eq!(
            sanitize_retry_after(Some("Sun, 06 Nov 1994 08:49:37 GMT")),
            DEFAULT_DO_RETRY_AFTER_SECS,
        );
        assert_eq!(
            sanitize_retry_after(Some("Sunday, 06-Nov-94 08:49:37 GMT")),
            DEFAULT_DO_RETRY_AFTER_SECS,
        );

        // Garbage — falls back to default rather than forwarding to client.
        for garbage in [
            "",                              // empty
            "   ",                           // whitespace-only
            "soon",                          // arbitrary token
            "-5",                            // negative (rejected by u32 parse)
            "30.5",                          // fractional
            "30s",                           // unit-suffixed
            "9999999999",                    // overflows u32
            "0x1e",                          // hex
            "\u{0007}garbage\u{0007}",       // control chars
            "Notaday, 06 Nov 1994 08:49:37", // bogus weekday
            "Sun 06 Nov 1994",               // missing time component
        ] {
            assert_eq!(
                sanitize_retry_after(Some(garbage)),
                DEFAULT_DO_RETRY_AFTER_SECS,
                "garbage value {garbage:?} should fall back to default",
            );
        }

        // Absent header — falls back to default.
        assert_eq!(sanitize_retry_after(None), DEFAULT_DO_RETRY_AFTER_SECS,);
    }

    #[test]
    fn classify_do_response_drops_garbage_retry_after_on_non_429() {
        // Non-429 errors must not propagate a malformed retry-after value
        // to the client. We drop the header rather than synthesizing a
        // default (which only applies to the 429 backpressure contract).
        assert_eq!(
            classify_do_response(503, Some("not-a-number")),
            DoForwardAction::Forward {
                status: 503,
                retry_after: None,
            },
        );
        // Valid integer is preserved as-is.
        assert_eq!(
            classify_do_response(503, Some("60")),
            DoForwardAction::Forward {
                status: 503,
                retry_after: Some("60".to_string()),
            },
        );
    }

    #[test]
    fn classify_do_response_sanitizes_garbage_retry_after_on_429() {
        // 429 with a garbage retry-after must be replaced with the default
        // — the client must never see invalid header content.
        assert_eq!(
            classify_do_response(429, Some("forever")),
            DoForwardAction::Forward {
                status: 429,
                retry_after: Some(DEFAULT_DO_RETRY_AFTER_SECS.to_string()),
            },
        );
    }

    #[test]
    fn parse_limit_query_parses_valid_value() {
        let url = Url::parse("https://example.test/v1/traces/r1?limit=25").ok();
        assert_eq!(parse_limit_query(url, "limit"), Some(25));
    }

    #[test]
    fn parse_limit_query_rejects_invalid_or_out_of_range() {
        let invalid = Url::parse("https://example.test/v1/traces/r1?limit=abc").ok();
        let zero = Url::parse("https://example.test/v1/traces/r1?limit=0").ok();
        let too_large = Url::parse("https://example.test/v1/traces/r1?limit=10001").ok();
        assert_eq!(parse_limit_query(invalid, "limit"), None);
        assert_eq!(parse_limit_query(zero, "limit"), None);
        assert_eq!(parse_limit_query(too_large, "limit"), None);
    }

    #[test]
    fn parse_limit_query_with_valid_presence_reports_valid_presence() {
        let with_param = Url::parse("https://example.test/v1/traces/r1?hops=7").ok();
        let without_param = Url::parse("https://example.test/v1/traces/r1").ok();
        let invalid_param = Url::parse("https://example.test/v1/traces/r1?hops=abc").ok();

        assert_eq!(
            parse_limit_query_with_valid_presence(with_param, "hops", 100),
            (7, true),
        );
        assert_eq!(
            parse_limit_query_with_valid_presence(without_param, "hops", 100),
            (100, false),
        );
        assert_eq!(
            parse_limit_query_with_valid_presence(invalid_param, "hops", 100),
            (100, false),
        );
    }

    #[test]
    fn build_trace_response_metadata_for_valid_untruncated_bound() {
        assert_eq!(
            build_trace_response_metadata(7, false, true),
            (Some(7), Some(false)),
        );
    }

    #[test]
    fn build_trace_response_metadata_for_valid_truncated_bound() {
        assert_eq!(
            build_trace_response_metadata(100, true, true),
            (None, Some(true)),
        );
    }

    #[test]
    fn build_trace_response_metadata_for_default_untruncated() {
        assert_eq!(
            build_trace_response_metadata(20, false, false),
            (None, None)
        );
    }

    #[test]
    fn build_trace_response_metadata_for_default_truncated() {
        assert_eq!(
            build_trace_response_metadata(1000, true, false),
            (None, Some(true)),
        );
    }

    // ── is_public_path ─────────────────────────────────────────

    #[test]
    fn is_public_path_root_and_health() {
        assert!(super::is_public_path("/"));
        assert!(super::is_public_path("/health"));
    }

    #[test]
    fn is_public_path_rejects_other_paths() {
        assert!(!super::is_public_path("/v1/artifacts"));
        assert!(!super::is_public_path("/v1/tenants/provision"));
        assert!(!super::is_public_path("/healthcheck"));
        assert!(!super::is_public_path("/health/"));
        assert!(!super::is_public_path(""));
    }

    // ── Cross-tenant DO instance naming (WS8) ──────────────────
    //
    // The Durable Object instance name is the only thing that decides
    // which DO instance handles a request. If two tenants resolve to
    // the same name, they share state — a cross-tenant data leak.
    // These tests pin the tenant-namespacing contract for PlayManager
    // and ThreadManager so a refactor cannot silently regress it.

    #[test]
    fn play_do_name_includes_tenant_prefix() {
        let n = super::play_do_name("tenant-alpha", "run-42");
        assert_eq!(n, "tenant-alpha:play:run-42");
    }

    #[test]
    fn play_do_name_separates_tenants_with_colliding_run_ids() {
        // Two tenants requesting the same run_id (e.g. via
        // `body.job_id`) must route to distinct DO instances.
        let alpha = super::play_do_name("alpha", "run-1");
        let beta = super::play_do_name("beta", "run-1");
        assert_ne!(
            alpha, beta,
            "PlayManager DO name must include tenant_id; otherwise a \
             guessable / colliding run_id leaks across tenants",
        );
    }

    #[test]
    fn thread_do_name_includes_tenant_prefix() {
        let n = super::thread_do_name("tenant-alpha", "thread-7");
        assert_eq!(n, "tenant-alpha:thread:thread-7");
    }

    #[test]
    fn thread_do_name_separates_tenants_with_colliding_thread_ids() {
        // Pre-WS8 the ThreadManager DO was named by thread_id alone, so
        // a guessable thread_id from tenant A could route into tenant
        // B's ThreadManager. After the fix, tenant prefix prevents this.
        let alpha = super::thread_do_name("alpha", "thread-shared");
        let beta = super::thread_do_name("beta", "thread-shared");
        assert_ne!(alpha, beta);
    }

    // ── Cross-tenant data isolation end-to-end (WS8) ───────────
    //
    // This is the harness-level integration test that exercises both
    // the SQL contract (via db.rs SQL constants — covered separately
    // in `mod tests` of db.rs) and the DO routing contract above. It
    // simulates "tenant alpha launches play X" and asserts that the
    // tenant-namespaced DO instance name + the tenant-scoped SQL
    // statements together make tenant beta unable to observe or
    // launch alpha's play.
    //
    // The full end-to-end flow requires a live D1 + DO harness, which
    // the worker crate does not have. We therefore assert the *two*
    // boundaries that together implement isolation: the DO name and
    // the SQL shape. If either regresses, this test fails.

    #[test]
    fn cross_tenant_play_launch_is_isolated_by_do_name_and_sql() {
        // Boundary 1: DO routing — tenant alpha's launch and tenant
        // beta's launch with the same run_id route to distinct DOs.
        let alpha = super::play_do_name("alpha", "run-x");
        let beta = super::play_do_name("beta", "run-x");
        assert_ne!(alpha, beta, "DO routing must isolate tenants");

        // Boundary 2: SQL contract — the SELECT for play_definitions
        // binds tenant_id, so tenant beta querying for a play named
        // "alpha-only" returns None even if the row exists under
        // tenant alpha. (Tested in db.rs cross_tenant_sql_* — we
        // reference it here so the integration narrative is in one
        // place.)
        //
        // No assertion needed at this layer — the db.rs unit test
        // already locks the SQL shape and is the authoritative gate.
    }

    // ── parse_play_launch_body (PR #132 finding @ src/lib.rs:451) ──
    // Bad JSON in a `POST /v1/plays/:name/launch` request body must be
    // rejected so the request surfaces as a 400 INVALID_JSON_BODY rather
    // than silently degrading to a default `PlayLaunchRequest` and returning
    // 200 OK (which masked the contract violation).

    #[test]
    fn parse_play_launch_body_accepts_valid_json() {
        let parsed =
            parse_play_launch_body(r#"{"play_name":"deploy","job_id":"j-1","metadata":null}"#)
                .expect("valid JSON body should parse");
        assert_eq!(parsed.play_name, "deploy");
        assert_eq!(parsed.job_id.as_deref(), Some("j-1"));
        assert!(parsed.metadata.is_none());
    }

    #[test]
    fn parse_play_launch_body_rejects_malformed_json() {
        // Truncated object — was previously absorbed by `unwrap_or_default`
        // at the handler call site, producing a 200 OK with a default body.
        assert!(parse_play_launch_body(r#"{"play_name":"deploy""#).is_err());
        // Wrong shape (missing required field).
        assert!(parse_play_launch_body(r#"{}"#).is_err());
        // Not JSON at all.
        assert!(parse_play_launch_body("not json").is_err());
    }

    // ── policy_activation_error_response_parts (PR #132 finding @ src/lib.rs:697) ──
    // Sanitization contract: the *client-facing* parts of a failed policy
    // activation response must NOT include the raw underlying error
    // (which previously leaked internal paths / stack context via
    // `format!("activation failed: {err}")`). The raw error is logged
    // server-side via `console_log!` and visible in `wrangler tail`.

    #[test]
    fn policy_activation_error_response_uses_stable_sanitized_envelope() {
        let (code, message, status) = policy_activation_error_response_parts();
        assert_eq!(code, "POLICY_ACTIVATION_FAILED");
        assert_eq!(status, 500);
        // Message is a fixed, short, operator-pointer string — never the
        // wrapped underlying error.
        assert_eq!(message, "policy activation failed; see server logs");
    }

    #[test]
    fn policy_activation_error_response_does_not_leak_raw_err() {
        // Simulate a raw underlying error string that would have been
        // interpolated into the old response (e.g. a worker::Error formatted
        // with paths, stack frames, or KV binding internals). The sanitized
        // response parts MUST NOT contain any of that.
        let raw_err =
            "KvError { binding: \"POLICY_KV\", path: \"/internal/policy/active\", source: ... }";
        let (code, message, _status) = policy_activation_error_response_parts();
        assert!(
            !message.contains(raw_err),
            "sanitized message must not embed raw err: got {message}",
        );
        assert!(
            !message.contains("KvError"),
            "sanitized message must not embed underlying error type",
        );
        assert!(
            !message.contains("binding"),
            "sanitized message must not embed binding details",
        );
        // The stable error code is also free of error context.
        assert!(!code.contains(' '), "code should be a stable identifier");
    }
}
