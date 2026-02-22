use serde::Serialize;
use worker::*;

mod db;
mod models;
mod policy;
mod storage;
mod tenant;

#[derive(Serialize)]
struct HealthResponse<'a> {
    service: &'a str,
    status: &'a str,
    mission: &'a str,
}

const MAX_ARTIFACT_BYTES: usize = 10 * 1024 * 1024;

fn is_public_path(path: &str) -> bool {
    path == "/" || path == "/health"
}

fn request_path(req: &Request) -> Result<String> {
    Ok(req.url()?.path().to_string())
}

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();

    let path = request_path(&req)?;
    if !is_public_path(&path) {
        let tenant_ctx = match tenant::tenant_from_request(&req) {
            Ok(ctx) => ctx,
            Err(_) => return Response::error("missing or invalid tenant context", 401),
        };
        if tenant::authorize(&tenant_ctx, req.method(), &path).is_err() {
            return Response::error("forbidden by tenant role policy", 403);
        }
    }

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
            let limit = params
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(50u32)
                .min(200);
            let d1 = ctx.env.d1("DB")?;
            let runs = db::list_runs(&d1, &tenant_ctx.tenant_id, repo, limit).await?;
            Response::from_json(&serde_json::json!({ "runs": runs }))
        })
        .get_async("/v1/runs/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx.param("id").unwrap().to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_run(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(run) => Response::from_json(&run),
                None => Response::error("run not found", 404),
            }
        })
        // ── WS2 Tasks (run-scoped, D1-backed) ───────────────
        .post_async("/v1/runs/:run_id/tasks", |mut req, ctx| async move {
            let run_id = ctx.param("run_id").unwrap().to_string();
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
            let run_id = ctx.param("run_id").unwrap().to_string();
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
            let body: models::UpsertMemoryItemRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            let expires_at = db::upsert_memory_item(&d1, &id, &body).await?;
            Response::from_json(&models::MemoryItemCreated {
                id,
                status: "indexed".into(),
                expires_at,
            })
        })
        .post_async("/v1/memory/retrieve", |mut req, ctx| async move {
            let body: models::RetrieveMemoryRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let response = db::retrieve_memory(&d1, &body).await?;
            Response::from_json(&response)
        })
        .post_async("/v1/memory/context-pack", |mut req, ctx| async move {
            let body: models::ContextPackRequest = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let response = db::build_context_pack(&d1, &body).await?;
            Response::from_json(&response)
        })
        .post_async("/v1/memory/:id/retire", |_req, ctx| async move {
            let id = match ctx.param("id") {
                Some(k) => k.to_string(),
                None => return Response::error("missing memory id", 400),
            };
            let d1 = ctx.env.d1("DB")?;
            let retired = db::retire_memory_item(&d1, &id).await?;
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
            let response = db::run_memory_gc(&d1, &body).await?;
            Response::from_json(&response)
        })
        .post_async("/v1/memory/retrieval-feedback", |mut req, ctx| async move {
            let body: models::RetrievalFeedback = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            db::record_retrieval_feedback(&d1, &body).await?;
            Response::from_json(&models::RetrievalFeedbackAck { recorded: true })
        })
        .get_async("/v1/memory/eval/summary", |_req, ctx| async move {
            let d1 = ctx.env.d1("DB")?;
            let summary = db::memory_eval_summary(&d1).await?;
            Response::from_json(&summary)
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
            let d1 = ctx.env.d1("DB")?;
            let rules = db::list_policy_rules(&d1, &tenant_ctx.tenant_id).await?;
            let responses: Vec<_> = rules.into_iter().map(|r| r.into_response()).collect();
            Response::from_json(&serde_json::json!({ "rules": responses }))
        })
        .get_async("/v1/policies/rules/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx.param("id").unwrap().to_string();
            let d1 = ctx.env.d1("DB")?;
            match db::get_policy_rule(&d1, &tenant_ctx.tenant_id, &id).await? {
                Some(rule) => Response::from_json(&rule.into_response()),
                None => Response::error("rule not found", 404),
            }
        })
        .patch_async("/v1/policies/rules/:id", |mut req, ctx| async move {
            let id = ctx.param("id").unwrap().to_string();
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
            let id = ctx.param("id").unwrap().to_string();
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
                Err(err) => Response::error(format!("activation failed: {err}"), 500),
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
            db::create_task(&d1, &tenant_ctx.tenant_id, &id, &body).await?;
            Response::from_json(&models::TaskCreated {
                id,
                status: "pending".into(),
            })
        })
        .get_async("/mcp/task/next", |req, ctx| async move {
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
            let caps_str = params.get("cap").cloned().unwrap_or_default();
            let caps: Vec<&str> = if caps_str.is_empty() {
                vec![]
            } else {
                caps_str.split(',').collect()
            };

            let d1 = ctx.env.d1("DB")?;
            match db::claim_next_task(&d1, &tenant_ctx.tenant_id, &agent_id, &caps).await? {
                Some(task) => Response::from_json(&task),
                None => Ok(Response::empty()?.with_status(204)),
            }
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

            let d1 = ctx.env.d1("DB")?;
            let updated =
                db::heartbeat_task(&d1, &tenant_ctx.tenant_id, &task_id, &agent_id).await?;
            if updated {
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
            let d1 = ctx.env.d1("DB")?;
            let updated =
                db::complete_task(&d1, &tenant_ctx.tenant_id, &task_id, body.result.as_ref())
                    .await?;
            if updated {
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
            let d1 = ctx.env.d1("DB")?;
            let new_status =
                db::fail_task(&d1, &tenant_ctx.tenant_id, &task_id, &body.error).await?;
            Response::from_json(&serde_json::json!({ "status": new_status }))
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
            let d1 = ctx.env.d1("DB")?;
            let agents = db::list_agents(&d1, &tenant_ctx.tenant_id).await?;
            Response::from_json(&serde_json::json!({ "agents": agents }))
        })
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
                let thread_id = ctx.param("thread_id").unwrap().to_string();
                let d1 = ctx.env.d1("DB")?;
                match db::get_latest_checkpoint(&d1, &tenant_ctx.tenant_id, &thread_id).await? {
                    Some(row) => Response::from_json(&row.into_checkpoint()),
                    None => Response::error("no checkpoint found", 404),
                }
            },
        )
        .get_async("/v1/checkpoints/:id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let id = ctx.param("id").unwrap().to_string();
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
            let body: models::CreateMemory = req.json().await?;
            let d1 = ctx.env.d1("DB")?;
            let id = generate_id()?;
            db::create_memory(&d1, &id, &body).await?;
            Response::from_json(&models::MemoryCreated {
                id,
                thread_id: body.thread_id,
                scope: body.scope,
                key: body.key,
            })
        })
        .get_async("/v1/memory/threads/:thread_id", |req, ctx| async move {
            let thread_id = match ctx.param("thread_id") {
                Some(t) => t.to_string(),
                None => return Response::error("missing thread_id", 400),
            };
            let limit = parse_limit_query(req.url().ok(), "limit").unwrap_or(100);
            let d1 = ctx.env.d1("DB")?;
            let memories = db::list_memories_for_thread(&d1, &thread_id, limit).await?;
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
                let memories = db::list_memories_for_thread(&d1, &thread_id, max_items).await?;
                Response::from_json(&serde_json::json!({
                    "thread_id": thread_id,
                    "checkpoint": checkpoint.map(|r| r.into_checkpoint()),
                    "memories": memories,
                }))
            },
        )
        // ── Traces / Provenance (WS3: issue #43) ──────────────
        .get_async("/v1/traces/:run_id", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let run_id = match ctx.param("run_id") {
                Some(r) => r.to_string(),
                None => return Response::error("missing run_id", 400),
            };
            let limit =
                parse_limit_query(req.url().ok(), "limit").unwrap_or(db::TRACE_DEFAULT_LIMIT);
            let d1 = ctx.env.d1("DB")?;
            // Fetch limit+1 to detect truncation without a separate COUNT query
            let mut events =
                db::get_trace_for_run(&d1, &tenant_ctx.tenant_id, &run_id, limit + 1).await?;
            let truncated = events.len() > limit as usize;
            if truncated {
                events.truncate(limit as usize);
            }
            Response::from_json(&models::TraceResponse {
                run_id,
                events,
                total: None,
                truncated: Some(truncated),
            })
        })
        .get_async("/v1/traces/:run_id/lineage", |req, ctx| async move {
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let run_id = match ctx.param("run_id") {
                Some(r) => r.to_string(),
                None => return Response::error("missing run_id", 400),
            };
            let limit = parse_limit_query(req.url().ok(), "hops").unwrap_or(100);
            let d1 = ctx.env.d1("DB")?;
            let events = db::get_trace_for_run(&d1, &tenant_ctx.tenant_id, &run_id, limit).await?;
            Response::from_json(&models::TraceResponse {
                run_id,
                events,
                total: None,
                truncated: None,
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
            if direction != "forward" && direction != "backward" {
                return Response::error("invalid direction: must be 'forward' or 'backward'", 400);
            }
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
            let tenant_ctx = tenant::tenant_from_request(&req)?;
            let d1 = ctx.env.d1("DB")?;
            let now = js_sys::Date::new_0().to_iso_string().as_string().unwrap();

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

                // Note: silver promotion is done synchronously in POST /v1/graph-events.
                // The queue consumer handles only causality edges and gold layer summaries.

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
