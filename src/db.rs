use crate::models;
use crate::policy::RiskLevel;
use std::collections::{HashMap, HashSet};
use wasm_bindgen::JsValue;
use worker::*;

fn now_iso() -> String {
    js_sys::Date::new_0().to_iso_string().as_string().unwrap()
}

fn opt_str(s: &Option<String>) -> JsValue {
    match s {
        Some(s) => JsValue::from_str(s),
        None => JsValue::NULL,
    }
}

fn opt_json(v: &Option<serde_json::Value>) -> JsValue {
    match v {
        Some(v) => JsValue::from_str(&serde_json::to_string(v).unwrap()),
        None => JsValue::NULL,
    }
}

// ── Tasks ───────────────────────────────────────────────────────

pub async fn create_task(db: &D1Database, id: &str, body: &models::CreateAgentTask) -> Result<()> {
    let now = now_iso();
    let max_retries = body.max_retries.unwrap_or(3);

    db.prepare(
        "INSERT INTO mcp_tasks (id, job_id, task_type, priority, status, params, graph_ref, play_id, parent_task_id, max_retries, created_at)
         VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.job_id),
        JsValue::from_str(&body.task_type),
        JsValue::from(body.priority),
        opt_json(&body.params),
        opt_str(&body.graph_ref),
        opt_str(&body.play_id),
        opt_str(&body.parent_task_id),
        JsValue::from(max_retries),
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;

    Ok(())
}

pub async fn claim_next_task(
    db: &D1Database,
    agent_id: &str,
    capabilities: &[&str],
) -> Result<Option<models::AgentTask>> {
    let now = now_iso();
    let lease = lease_time(300);

    // Build capability filter using parameterized placeholders (no string interpolation)
    let result: Option<TaskIdRow> = if capabilities.is_empty() {
        db.prepare(
            "SELECT id FROM mcp_tasks WHERE status = 'pending' ORDER BY priority DESC, created_at ASC LIMIT 1",
        )
        .bind(&[])?
        .first(None)
        .await?
    } else {
        let placeholders: Vec<String> = (1..=capabilities.len())
            .map(|i| format!("?{}", i))
            .collect();
        let query = format!(
            "SELECT id FROM mcp_tasks WHERE status = 'pending' AND task_type IN ({}) ORDER BY priority DESC, created_at ASC LIMIT 1",
            placeholders.join(", ")
        );
        let bindings: Vec<JsValue> = capabilities.iter().map(|c| JsValue::from_str(c)).collect();
        db.prepare(&query).bind(&bindings)?.first(None).await?
    };
    let task_id = match result {
        Some(row) => row.id,
        None => return Ok(None),
    };

    // Claim it atomically: only succeeds if still pending
    db.prepare(
        "UPDATE mcp_tasks SET status = 'running', agent_id = ?1, lease_expires_at = ?2 WHERE id = ?3 AND status = 'pending'",
    )
    .bind(&[
        JsValue::from_str(agent_id),
        JsValue::from_str(&lease),
        JsValue::from_str(&task_id),
    ])?
    .run()
    .await?;

    // Update agent heartbeat
    db.prepare("UPDATE agents SET last_heartbeat = ?1 WHERE id = ?2")
        .bind(&[JsValue::from_str(&now), JsValue::from_str(agent_id)])?
        .run()
        .await?;

    // Fetch full task
    let task: Option<TaskRow> = db
        .prepare("SELECT * FROM mcp_tasks WHERE id = ?1 AND agent_id = ?2")
        .bind(&[JsValue::from_str(&task_id), JsValue::from_str(agent_id)])?
        .first(None)
        .await?;

    Ok(task.map(|r| r.into_agent_task()))
}

pub async fn heartbeat_task(db: &D1Database, task_id: &str, agent_id: &str) -> Result<bool> {
    let lease = lease_time(300);
    let result: D1Result = db
        .prepare(
            "UPDATE mcp_tasks SET lease_expires_at = ?1 WHERE id = ?2 AND agent_id = ?3 AND status = 'running'",
        )
        .bind(&[
            JsValue::from_str(&lease),
            JsValue::from_str(task_id),
            JsValue::from_str(agent_id),
        ])?
        .run()
        .await?;

    let changed = result
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);
    Ok(changed)
}

pub async fn complete_task(
    db: &D1Database,
    task_id: &str,
    result_val: Option<&serde_json::Value>,
) -> Result<bool> {
    let now = now_iso();
    let result_json = result_val.map(|v| serde_json::to_string(v).unwrap());
    let res: D1Result = db
        .prepare(
            "UPDATE mcp_tasks SET status = 'completed', result = ?1, completed_at = ?2 WHERE id = ?3 AND status = 'running'",
        )
        .bind(&[
            match &result_json {
                Some(s) => JsValue::from_str(s),
                None => JsValue::NULL,
            },
            JsValue::from_str(&now),
            JsValue::from_str(task_id),
        ])?
        .run()
        .await?;

    let changed = res
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);
    Ok(changed)
}

pub async fn fail_task(db: &D1Database, task_id: &str, error: &str) -> Result<String> {
    let now = now_iso();

    // Check retry eligibility
    let task: Option<RetryRow> = db
        .prepare(
            "SELECT retry_count, max_retries FROM mcp_tasks WHERE id = ?1 AND status = 'running'",
        )
        .bind(&[JsValue::from_str(task_id)])?
        .first(None)
        .await?;

    let (new_status, new_retry) = match task {
        Some(row) if row.retry_count < row.max_retries => ("pending", row.retry_count + 1),
        _ => ("failed", 0),
    };

    let result_json = serde_json::json!({ "error": error }).to_string();

    db.prepare(
        "UPDATE mcp_tasks SET status = ?1, retry_count = CASE WHEN ?1 = 'pending' THEN ?2 ELSE retry_count END, result = ?3, agent_id = CASE WHEN ?1 = 'pending' THEN NULL ELSE agent_id END, lease_expires_at = NULL, completed_at = CASE WHEN ?1 = 'failed' THEN ?4 ELSE NULL END WHERE id = ?5",
    )
    .bind(&[
        JsValue::from_str(new_status),
        JsValue::from(new_retry),
        JsValue::from_str(&result_json),
        JsValue::from_str(&now),
        JsValue::from_str(task_id),
    ])?
    .run()
    .await?;

    Ok(new_status.to_string())
}

// ── Agents ──────────────────────────────────────────────────────

pub async fn register_agent(db: &D1Database, id: &str, body: &models::RegisterAgent) -> Result<()> {
    let now = now_iso();
    let caps_json = serde_json::to_string(&body.capabilities).unwrap();

    db.prepare(
        "INSERT INTO agents (id, name, capabilities, endpoint, last_heartbeat, status, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.name),
        JsValue::from_str(&caps_json),
        opt_str(&body.endpoint),
        JsValue::from_str(&now),
        opt_json(&body.metadata),
    ])?
    .run()
    .await?;

    Ok(())
}

pub async fn list_agents(db: &D1Database) -> Result<Vec<models::Agent>> {
    let result: D1Result = db
        .prepare("SELECT * FROM agents WHERE status = 'active' ORDER BY name")
        .bind(&[])?
        .all()
        .await?;

    let rows: Vec<AgentRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_agent()).collect())
}

// ── Checkpoints ─────────────────────────────────────────────────

pub async fn create_checkpoint(
    db: &D1Database,
    id: &str,
    body: &models::CreateCheckpoint,
    r2_key: &str,
    size: i64,
) -> Result<()> {
    let now = now_iso();

    db.prepare(
        "INSERT INTO checkpoints (id, thread_id, node_id, parent_id, state_r2_key, state_size_bytes, metadata, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.thread_id),
        JsValue::from_str(&body.node_id),
        opt_str(&body.parent_id),
        JsValue::from_str(r2_key),
        JsValue::from(size as f64),
        opt_json(&body.metadata),
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;

    Ok(())
}

pub async fn get_latest_checkpoint(
    db: &D1Database,
    thread_id: &str,
) -> Result<Option<CheckpointRow>> {
    db.prepare("SELECT * FROM checkpoints WHERE thread_id = ?1 ORDER BY created_at DESC LIMIT 1")
        .bind(&[JsValue::from_str(thread_id)])?
        .first(None)
        .await
}

pub async fn get_checkpoint_by_id(db: &D1Database, id: &str) -> Result<Option<CheckpointRow>> {
    db.prepare("SELECT * FROM checkpoints WHERE id = ?1")
        .bind(&[JsValue::from_str(id)])?
        .first(None)
        .await
}

pub async fn delete_checkpoint(db: &D1Database, id: &str) -> Result<bool> {
    let res: D1Result = db
        .prepare("DELETE FROM checkpoints WHERE id = ?1")
        .bind(&[JsValue::from_str(id)])?
        .run()
        .await?;

    let changed = res
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);
    Ok(changed)
}

// ── Memory (WS5: #45) ──────────────────────────────────────────

pub async fn create_memory(db: &D1Database, id: &str, body: &models::CreateMemory) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO memory (id, run_id, thread_id, scope, key, ref_type, ref_id, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(&[
        JsValue::from_str(id),
        opt_str(&body.run_id),
        JsValue::from_str(&body.thread_id),
        JsValue::from_str(&body.scope),
        JsValue::from_str(&body.key),
        JsValue::from_str(&body.ref_type),
        JsValue::from_str(&body.ref_id),
        JsValue::from_str(&now),
        opt_str(&body.expires_at),
    ])?
    .run()
    .await?;
    Ok(())
}

/// List memories for a thread, recency-ordered. Limit applied (default 100). Excludes expired if expires_at < now.
pub async fn list_memories_for_thread(
    db: &D1Database,
    thread_id: &str,
    limit: u32,
) -> Result<Vec<models::Memory>> {
    let now = now_iso();
    let result: D1Result = db
        .prepare(
            "SELECT * FROM memory WHERE thread_id = ?1 AND (expires_at IS NULL OR expires_at > ?2) ORDER BY created_at DESC LIMIT ?3",
        )
        .bind(&[
            JsValue::from_str(thread_id),
            JsValue::from_str(&now),
            JsValue::from(limit),
        ])?
        .all()
        .await?;
    let rows: Vec<MemoryRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_memory()).collect())
}

// ── Events Bronze ───────────────────────────────────────────────

pub async fn insert_events_bronze(
    db: &D1Database,
    events: &[(String, &models::GraphEvent, String)],
) -> Result<()> {
    let mut stmts = Vec::with_capacity(events.len());
    for (id, evt, now) in events {
        let payload_json = evt
            .payload
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap());

        let stmt = db
            .prepare(
                "INSERT INTO events_bronze (id, run_id, thread_id, event_type, node_id, actor, payload, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .bind(&[
                JsValue::from_str(id),
                opt_str(&evt.run_id),
                opt_str(&evt.thread_id),
                JsValue::from_str(&evt.event_type),
                opt_str(&evt.node_id),
                opt_str(&evt.actor),
                match &payload_json {
                    Some(s) => JsValue::from_str(s),
                    None => JsValue::NULL,
                },
                JsValue::from_str(now),
            ])?;
        stmts.push(stmt);
    }

    db.batch(stmts).await?;
    Ok(())
}

/// Default max events per trace query (avoids unbounded result sets).
pub const TRACE_DEFAULT_LIMIT: u32 = 1000;

/// Fetch trace slice for a run (ordered by created_at). WS3 provenance. Limited by `limit` (default TRACE_DEFAULT_LIMIT).
pub async fn get_trace_for_run(
    db: &D1Database,
    run_id: &str,
    limit: u32,
) -> Result<Vec<models::TraceEvent>> {
    let result: D1Result = db
        .prepare(
            "SELECT id, run_id, thread_id, event_type, node_id, actor, payload, created_at
             FROM events_bronze WHERE run_id = ?1 ORDER BY created_at ASC LIMIT ?2",
        )
        .bind(&[JsValue::from_str(run_id), JsValue::from(limit)])?
        .all()
        .await?;

    let rows: Vec<TraceEventRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_trace_event()).collect())
}

/// Insert into silver layer (sync promotion). Each row has its own id, bronze_id FK, entity_refs from event.
pub async fn insert_events_silver(
    db: &D1Database,
    events: &[(String, String, &models::GraphEvent, String)], // (silver_id, bronze_id, evt, created_at)
    normalized_at: &str,
) -> Result<()> {
    let mut stmts = Vec::with_capacity(events.len());
    for (silver_id, bronze_id, evt, created_at) in events {
        let payload_json = evt
            .payload
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap());
        let entity_refs_json = build_entity_refs(evt);

        let stmt = db
            .prepare(
                "INSERT INTO events_silver (id, bronze_id, run_id, thread_id, event_type, node_id, actor, payload, created_at, normalized_at, entity_refs)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .bind(&[
                JsValue::from_str(silver_id),
                JsValue::from_str(bronze_id),
                opt_str(&evt.run_id),
                opt_str(&evt.thread_id),
                JsValue::from_str(&evt.event_type),
                opt_str(&evt.node_id),
                opt_str(&evt.actor),
                match &payload_json {
                    Some(s) => JsValue::from_str(s),
                    None => JsValue::NULL,
                },
                JsValue::from_str(created_at),
                JsValue::from_str(normalized_at),
                match &entity_refs_json {
                    Some(s) => JsValue::from_str(s),
                    None => JsValue::NULL,
                },
            ])?;
        stmts.push(stmt);
    }

    db.batch(stmts).await?;
    Ok(())
}

fn build_entity_refs(evt: &models::GraphEvent) -> Option<String> {
    let mut obj = serde_json::Map::new();
    if let Some(ref r) = evt.run_id {
        obj.insert("run_id".into(), serde_json::Value::String(r.clone()));
    }
    if let Some(ref t) = evt.thread_id {
        obj.insert("thread_id".into(), serde_json::Value::String(t.clone()));
    }
    if let Some(ref n) = evt.node_id {
        obj.insert("node_id".into(), serde_json::Value::String(n.clone()));
    }
    if obj.is_empty() {
        return None;
    }
    serde_json::to_string(&serde_json::Value::Object(obj)).ok()
}

// ── WS2 Domain: Runs ─────────────────────────────────────────────

pub async fn create_run(db: &D1Database, id: &str, body: &models::CreateRun) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO runs (id, repo, status, trigger, actor, created_at, updated_at, metadata)
         VALUES (?1, ?2, 'created', ?3, ?4, ?5, ?5, ?6)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.repo),
        opt_str(&body.trigger),
        JsValue::from_str(body.actor.as_deref().unwrap_or("unknown")),
        JsValue::from_str(&now),
        opt_json(&body.metadata),
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn list_runs(
    db: &D1Database,
    repo: Option<&str>,
    limit: u32,
) -> Result<Vec<RunResponse>> {
    let (query, bindings): (String, Vec<JsValue>) = match repo {
        Some(r) => (
            "SELECT * FROM runs WHERE repo = ?1 ORDER BY created_at DESC LIMIT ?2".into(),
            vec![JsValue::from_str(r), JsValue::from(limit)],
        ),
        None => (
            "SELECT * FROM runs ORDER BY created_at DESC LIMIT ?1".into(),
            vec![JsValue::from(limit)],
        ),
    };
    let result: D1Result = db.prepare(&query).bind(&bindings)?.all().await?;
    let rows: Vec<RunRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_run_response()).collect())
}

pub async fn get_run(db: &D1Database, id: &str) -> Result<Option<RunResponse>> {
    let row: Option<RunRow> = db
        .prepare("SELECT * FROM runs WHERE id = ?1")
        .bind(&[JsValue::from_str(id)])?
        .first(None)
        .await?;
    Ok(row.map(|r| r.into_run_response()))
}

// ── WS5 Retrieval + Memory Federation ───────────────────────────

pub async fn upsert_memory_item(
    db: &D1Database,
    id: &str,
    body: &models::UpsertMemoryItemRequest,
) -> Result<Option<String>> {
    let now = now_iso();
    let tags_json = serde_json::to_string(&body.tags).unwrap_or_else(|_| "[]".to_string());
    let metadata_json = body
        .metadata
        .as_ref()
        .map(|m| serde_json::to_string(m).unwrap_or_else(|_| "{}".to_string()));
    let source_created_at = body
        .source_created_at
        .clone()
        .unwrap_or_else(|| now.clone());
    let expires_at = body
        .ttl_seconds
        .map(|ttl| add_seconds_to_now(ttl.max(0) as u64));
    let kind = serde_json::to_string(&body.kind)
        .unwrap_or_else(|_| "\"context\"".to_string())
        .trim_matches('"')
        .to_string();
    let conflict_version = body.conflict_version.unwrap_or(1).max(1);

    db.prepare(
        "INSERT INTO memory_index (
            id, repo, kind, run_id, task_id, thread_id, checkpoint_id, artifact_key, title, summary,
            tags, content_ref, metadata, success_rate, source_created_at, indexed_at, last_accessed_at,
            access_count, status, unsafe_reason, expires_at, conflict_key, conflict_version
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, ?15, ?16, NULL,
            0, 'active', ?17, ?18, ?19, ?20
        )",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.repo),
        JsValue::from_str(&kind),
        opt_str(&body.run_id),
        opt_str(&body.task_id),
        opt_str(&body.thread_id),
        opt_str(&body.checkpoint_id),
        opt_str(&body.artifact_key),
        opt_str(&body.title),
        JsValue::from_str(&body.summary),
        JsValue::from_str(&tags_json),
        opt_str(&body.content_ref),
        match &metadata_json {
            Some(s) => JsValue::from_str(s),
            None => JsValue::NULL,
        },
        match body.success_rate {
            Some(v) => JsValue::from_f64(v),
            None => JsValue::NULL,
        },
        JsValue::from_str(&source_created_at),
        JsValue::from_str(&now),
        opt_str(&body.unsafe_reason),
        match &expires_at {
            Some(s) => JsValue::from_str(s),
            None => JsValue::NULL,
        },
        opt_str(&body.conflict_key),
        JsValue::from(conflict_version),
    ])?
    .run()
    .await?;

    Ok(expires_at)
}

// ── WS2 Domain: Tasks (run-scoped) ──────────────────────────────

pub async fn create_ws2_task(
    db: &D1Database,
    id: &str,
    run_id: &str,
    body: &models::CreateTask,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO tasks (id, run_id, plan_id, name, status, actor, created_at, updated_at, metadata)
         VALUES (?1, ?2, ?3, ?4, 'created', ?5, ?6, ?6, ?7)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(run_id),
        opt_str(&body.plan_id),
        JsValue::from_str(&body.name),
        opt_str(&body.actor),
        JsValue::from_str(&now),
        opt_json(&body.metadata),
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn list_ws2_tasks(db: &D1Database, run_id: &str) -> Result<Vec<Ws2TaskResponse>> {
    let result: D1Result = db
        .prepare("SELECT * FROM tasks WHERE run_id = ?1 ORDER BY created_at ASC")
        .bind(&[JsValue::from_str(run_id)])?
        .all()
        .await?;
    let rows: Vec<Ws2TaskRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_task_response()).collect())
}

// ── WS2 Domain: Plans ────────────────────────────────────────────

pub async fn create_plan(db: &D1Database, id: &str, body: &models::CreatePlan) -> Result<()> {
    let now = now_iso();
    let task_ids_json = body
        .task_ids
        .as_ref()
        .map(|ids| serde_json::to_string(ids).unwrap())
        .unwrap_or_else(|| "[]".into());

    db.prepare(
        "INSERT INTO plans (id, run_id, name, status, task_ids, created_at, updated_at, metadata)
         VALUES (?1, ?2, ?3, 'created', ?4, ?5, ?5, ?6)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.run_id),
        JsValue::from_str(&body.name),
        JsValue::from_str(&task_ids_json),
        JsValue::from_str(&now),
        opt_json(&body.metadata),
    ])?
    .run()
    .await?;
    Ok(())
}

// ── WS2 Domain: Tool Calls ──────────────────────────────────────

pub async fn record_tool_call(
    db: &D1Database,
    id: &str,
    body: &models::RecordToolCall,
) -> Result<()> {
    let now = now_iso();
    let input_json = serde_json::to_string(&body.input).unwrap();

    db.prepare(
        "INSERT INTO tool_calls (id, run_id, task_id, tool_name, input, output, status, duration_ms, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'created', ?7, ?8)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.run_id),
        opt_str(&body.task_id),
        JsValue::from_str(&body.tool_name),
        JsValue::from_str(&input_json),
        opt_json(&body.output),
        match body.duration_ms {
            Some(ms) => JsValue::from(ms as f64),
            None => JsValue::NULL,
        },
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn retrieve_memory(
    db: &D1Database,
    req: &models::RetrieveMemoryRequest,
) -> Result<models::RetrieveMemoryResponse> {
    let start = js_sys::Date::now();
    let now = now_iso();
    let now_ms = js_sys::Date::parse(&now);
    let query_id = random_hex_id()?;

    let mut repos = vec![req.repo.clone()];
    for r in &req.related_repos {
        if !repos.iter().any(|x| x == r) {
            repos.push(r.clone());
        }
    }

    let mut bind: Vec<JsValue> = vec![];
    let mut where_parts: Vec<String> = vec![];
    let mut idx = 1usize;

    where_parts.push("status = 'active'".to_string());

    let repo_clause = (0..repos.len())
        .map(|_| {
            let p = format!("?{idx}");
            idx += 1;
            p
        })
        .collect::<Vec<_>>()
        .join(", ");
    where_parts.push(format!("repo IN ({repo_clause})"));
    for repo in &repos {
        bind.push(JsValue::from_str(repo));
    }

    if let Some(thread_id) = &req.thread_id {
        where_parts.push(format!("thread_id = ?{idx}"));
        bind.push(JsValue::from_str(thread_id));
        idx += 1;
    }
    if let Some(run_id) = &req.run_id {
        where_parts.push(format!("run_id = ?{idx}"));
        bind.push(JsValue::from_str(run_id));
        idx += 1;
    }
    if let Some(task_id) = &req.task_id {
        where_parts.push(format!("task_id = ?{idx}"));
        bind.push(JsValue::from_str(task_id));
        idx += 1;
    }

    if !req.include_unsafe {
        where_parts.push("unsafe_reason IS NULL".to_string());
    }
    if !req.include_stale {
        where_parts.push(format!("(expires_at IS NULL OR expires_at > ?{idx})"));
        bind.push(JsValue::from_str(&now));
        idx += 1;
    }

    let qlike = format!("%{}%", req.query.to_lowercase());
    where_parts.push(format!(
        "(lower(summary) LIKE ?{idx} OR lower(COALESCE(title, '')) LIKE ?{} OR lower(COALESCE(tags, '')) LIKE ?{})",
        idx + 1,
        idx + 2
    ));
    bind.push(JsValue::from_str(&qlike));
    bind.push(JsValue::from_str(&qlike));
    bind.push(JsValue::from_str(&qlike));

    let sql = format!(
        "SELECT * FROM memory_index WHERE {} ORDER BY indexed_at DESC LIMIT 200",
        where_parts.join(" AND ")
    );

    let result: D1Result = db.prepare(&sql).bind(&bind)?.all().await?;
    let rows: Vec<MemoryIndexRow> = result.results()?;

    let mut stale_filtered = 0usize;
    let mut unsafe_filtered = 0usize;
    let mut conflict_filtered = 0usize;

    let latest_conflicts = latest_conflict_versions(&rows);

    let mut candidates = vec![];
    for row in rows {
        let stale = is_stale(&row, &now);
        let conflicted = is_conflicted(&row, &latest_conflicts);

        if stale && !req.include_stale {
            stale_filtered += 1;
            continue;
        }
        if row.unsafe_reason.is_some() && !req.include_unsafe {
            unsafe_filtered += 1;
            continue;
        }
        if conflicted && !req.include_conflicted {
            conflict_filtered += 1;
            continue;
        }

        let score = score_candidate(&row, &req.query, now_ms);
        let estimated_tokens = estimate_tokens(&row.title, &row.summary, &row.tags);
        candidates.push(models::MemoryCandidate {
            id: row.id,
            repo: row.repo,
            kind: row.kind,
            run_id: row.run_id,
            task_id: row.task_id,
            thread_id: row.thread_id,
            title: row.title,
            summary: row.summary,
            tags: parse_tags(&row.tags),
            content_ref: row.content_ref,
            success_rate: row.success_rate,
            stale,
            unsafe_reason: row.unsafe_reason,
            conflicted,
            estimated_tokens,
            score,
        });
    }

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_k = req.top_k.clamp(1, 50);
    let selected = candidates.into_iter().take(top_k).collect::<Vec<_>>();

    let elapsed = (js_sys::Date::now() - start).round() as i64;
    log_retrieval_query(
        db,
        &query_id,
        req,
        selected.len() as i64,
        elapsed,
        stale_filtered as i64,
        unsafe_filtered as i64,
        conflict_filtered as i64,
    )
    .await?;
    touch_memory_items(db, &selected).await?;

    Ok(models::RetrieveMemoryResponse {
        query_id,
        latency_ms: elapsed,
        total_candidates: selected.len() + stale_filtered + unsafe_filtered + conflict_filtered,
        returned: selected.len(),
        stale_filtered,
        unsafe_filtered,
        conflict_filtered,
        items: selected,
    })
}

pub async fn build_context_pack(
    db: &D1Database,
    req: &models::ContextPackRequest,
) -> Result<models::ContextPackResponse> {
    let retrieval = retrieve_memory(db, &req.retrieval).await?;
    let mut used = 0usize;
    let mut dropped = 0usize;
    let mut packed = vec![];
    for item in retrieval.items {
        if used + item.estimated_tokens > req.token_budget {
            dropped += 1;
            continue;
        }
        used += item.estimated_tokens;
        packed.push(item);
    }

    Ok(models::ContextPackResponse {
        query_id: retrieval.query_id,
        latency_ms: retrieval.latency_ms,
        token_budget: req.token_budget,
        used_tokens: used,
        dropped_due_to_budget: dropped,
        items: packed,
    })
}

pub async fn retire_memory_item(db: &D1Database, id: &str) -> Result<bool> {
    let now = now_iso();
    let res: D1Result = db
        .prepare("UPDATE memory_index SET status = 'retired', indexed_at = ?1 WHERE id = ?2")
        .bind(&[JsValue::from_str(&now), JsValue::from_str(id)])?
        .run()
        .await?;
    let changed = res
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);
    Ok(changed)
}

pub async fn run_memory_gc(
    db: &D1Database,
    req: &models::MemoryGcRequest,
) -> Result<models::MemoryGcResponse> {
    let now = now_iso();
    let limit = req.limit.clamp(1, 10_000) as i32;

    let rows: Vec<MemoryIdRow> = db
        .prepare(
            "SELECT id FROM memory_index
             WHERE (status = 'retired' OR (expires_at IS NOT NULL AND expires_at <= ?1))
             ORDER BY indexed_at ASC
             LIMIT ?2",
        )
        .bind(&[JsValue::from_str(&now), JsValue::from(limit)])?
        .all()
        .await?
        .results()?;

    let scanned = rows.len();
    if scanned == 0 {
        return Ok(models::MemoryGcResponse {
            scanned: 0,
            retired: 0,
            deleted: 0,
        });
    }

    let mut stmts = Vec::with_capacity(rows.len());
    for row in &rows {
        let stmt = db
            .prepare("DELETE FROM memory_index WHERE id = ?1")
            .bind(&[JsValue::from_str(&row.id)])?;
        stmts.push(stmt);
    }
    db.batch(stmts).await?;

    Ok(models::MemoryGcResponse {
        scanned,
        retired: 0,
        deleted: scanned,
    })
}

pub async fn record_retrieval_feedback(
    db: &D1Database,
    feedback: &models::RetrievalFeedback,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO memory_retrieval_feedback (
            query_id, run_id, task_id, success, first_pass_success, cache_hit, latency_ms, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&[
        JsValue::from_str(&feedback.query_id),
        opt_str(&feedback.run_id),
        opt_str(&feedback.task_id),
        JsValue::from(if feedback.success { 1 } else { 0 }),
        JsValue::from(if feedback.first_pass_success { 1 } else { 0 }),
        JsValue::from(if feedback.cache_hit { 1 } else { 0 }),
        match feedback.latency_ms {
            Some(v) => JsValue::from(v as f64),
            None => JsValue::NULL,
        },
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;
    Ok(())
}

// ── WS2 Domain: Releases ────────────────────────────────────────

pub async fn create_release(db: &D1Database, id: &str, body: &models::CreateRelease) -> Result<()> {
    let now = now_iso();
    let artifact_ids_json = body
        .artifact_ids
        .as_ref()
        .map(|ids| serde_json::to_string(ids).unwrap())
        .unwrap_or_else(|| "[]".into());

    db.prepare(
        "INSERT INTO releases (id, repo, version, run_id, artifact_ids, status, created_at, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, 'created', ?6, ?7)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.repo),
        JsValue::from_str(&body.version),
        JsValue::from_str(&body.run_id),
        JsValue::from_str(&artifact_ids_json),
        JsValue::from_str(&now),
        opt_json(&body.metadata),
    ])?
    .run()
    .await?;
    Ok(())
}

// ── WS2 Domain: Events (provenance) ─────────────────────────────

pub async fn ingest_event(db: &D1Database, id: &str, body: &models::IngestEvent) -> Result<()> {
    let now = now_iso();
    let entity_kind = body.entity_kind.as_deref().unwrap_or("event");
    let entity_id = body.entity_id.as_deref().unwrap_or(id);
    db.prepare(
        "INSERT INTO events (id, run_id, entity_kind, entity_id, event_type, actor, created_at, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.run_id),
        JsValue::from_str(entity_kind),
        JsValue::from_str(entity_id),
        JsValue::from_str(&body.event_type),
        JsValue::from_str(&body.actor),
        JsValue::from_str(&now),
        opt_json(&body.payload),
    ])?
    .run()
    .await?;
    Ok(())
}

// ── WS4 Domain: Policy Decisions ────────────────────────────────

pub async fn record_policy_check_detailed(
    db: &D1Database,
    id: &str,
    body: &models::PolicyCheckRequest,
    decision: &str,
    reason: &str,
    risk: RiskLevel,
    policy_version: &str,
    matched_rule: Option<&str>,
    escalation_id: Option<&str>,
    rate_limited: bool,
) -> Result<()> {
    let now = now_iso();
    let mut merged = body.context.clone().unwrap_or_else(|| serde_json::json!({}));
    if !merged.is_object() {
        merged = serde_json::json!({ "context": merged });
    }
    if let Some(obj) = merged.as_object_mut() {
        obj.insert(
            "risk_level".into(),
            serde_json::json!(format!("{:?}", risk).to_ascii_lowercase()),
        );
        obj.insert("policy_version".into(), serde_json::json!(policy_version));
        obj.insert("matched_rule".into(), serde_json::json!(matched_rule));
        obj.insert("escalation_id".into(), serde_json::json!(escalation_id));
        obj.insert("rate_limited".into(), serde_json::json!(rate_limited));
    }

    db.prepare(
        "INSERT INTO policy_decisions (id, run_id, action, actor, resource, decision, reason, created_at, context)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(&[
        JsValue::from_str(id),
        opt_str(&body.run_id),
        JsValue::from_str(&body.action),
        JsValue::from_str(&body.actor),
        opt_str(&body.resource),
        JsValue::from_str(decision),
        JsValue::from_str(reason),
        JsValue::from_str(&now),
        JsValue::from_str(&serde_json::to_string(&merged).unwrap_or_else(|_| "{}".into())),
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn create_policy_escalation(
    db: &D1Database,
    escalation_id: &str,
    decision_id: &str,
    action: &str,
    actor: &str,
    resource: Option<&str>,
    risk: RiskLevel,
    context: Option<&serde_json::Value>,
) -> Result<()> {
    let now = now_iso();
    let payload = context
        .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".into()))
        .unwrap_or_else(|| "{}".into());
    db.prepare(
        "INSERT INTO policy_escalations (id, decision_id, action, actor, resource, risk_level, status, context, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8)",
    )
    .bind(&[
        JsValue::from_str(escalation_id),
        JsValue::from_str(decision_id),
        JsValue::from_str(action),
        JsValue::from_str(actor),
        match resource {
            Some(r) => JsValue::from_str(r),
            None => JsValue::NULL,
        },
        JsValue::from_str(&format!("{:?}", risk).to_ascii_lowercase()),
        JsValue::from_str(&payload),
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;
    Ok(())
}

// ── WS4 Policy Rules Engine ──────────────────────────────────────

pub async fn create_policy_rule(
    db: &D1Database,
    id: &str,
    body: &models::CreatePolicyRule,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO policy_rules (id, name, action_pattern, resource_pattern, actor_pattern, risk_level, verdict, reason, priority, enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.name),
        JsValue::from_str(&body.action_pattern),
        JsValue::from_str(&body.resource_pattern),
        JsValue::from_str(&body.actor_pattern),
        JsValue::from_str(&body.risk_level),
        JsValue::from_str(&body.verdict),
        JsValue::from_str(&body.reason),
        JsValue::from(body.priority),
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn memory_eval_summary(db: &D1Database) -> Result<models::MemoryEvalSummary> {
    let row: Option<MemoryEvalRow> = db
        .prepare(
            "SELECT
                COUNT(*) AS total_queries,
                AVG(CASE WHEN cache_hit = 1 THEN 1.0 ELSE 0.0 END) AS cache_hit_rate,
                AVG(CASE WHEN success = 1 THEN 1.0 ELSE 0.0 END) AS success_rate,
                AVG(CASE WHEN first_pass_success = 1 THEN 1.0 ELSE 0.0 END) AS first_pass_success_rate
             FROM memory_retrieval_feedback",
        )
        .bind(&[])?
        .first(None)
        .await?;

    let p50: Option<PercentileRow> = db
        .prepare(
            "SELECT latency_ms
             FROM memory_retrieval_feedback
             WHERE latency_ms IS NOT NULL
             ORDER BY latency_ms
             LIMIT 1 OFFSET (SELECT CAST((COUNT(*) * 0.50) AS INTEGER) FROM memory_retrieval_feedback WHERE latency_ms IS NOT NULL)",
        )
        .bind(&[])?
        .first(None)
        .await?;
    let p95: Option<PercentileRow> = db
        .prepare(
            "SELECT latency_ms
             FROM memory_retrieval_feedback
             WHERE latency_ms IS NOT NULL
             ORDER BY latency_ms
             LIMIT 1 OFFSET (SELECT CAST((COUNT(*) * 0.95) AS INTEGER) FROM memory_retrieval_feedback WHERE latency_ms IS NOT NULL)",
        )
        .bind(&[])?
        .first(None)
        .await?;

    let summary = row.unwrap_or(MemoryEvalRow {
        total_queries: 0,
        cache_hit_rate: None,
        success_rate: None,
        first_pass_success_rate: None,
    });

    Ok(models::MemoryEvalSummary {
        total_queries: summary.total_queries,
        cache_hit_rate: summary.cache_hit_rate.unwrap_or(0.0),
        success_rate: summary.success_rate.unwrap_or(0.0),
        first_pass_success_rate: summary.first_pass_success_rate.unwrap_or(0.0),
        p50_latency_ms: p50.and_then(|p| p.latency_ms),
        p95_latency_ms: p95.and_then(|p| p.latency_ms),
    })
}

fn score_candidate(row: &MemoryIndexRow, query: &str, now_ms: f64) -> f64 {
    let query_tokens = tokenize(query);
    let text = format!(
        "{} {} {}",
        row.title.clone().unwrap_or_default(),
        row.summary,
        row.tags.clone().unwrap_or_default()
    );
    let text_tokens = tokenize(&text);
    let similarity = jaccard_similarity(&query_tokens, &text_tokens);

    let freshness = freshness_score(row, now_ms);
    let success = row.success_rate.unwrap_or(0.5).clamp(0.0, 1.0);
    let popularity = (row.access_count.max(0) as f64 / 20.0).min(1.0);

    (similarity * 0.55) + (freshness * 0.25) + (success * 0.15) + (popularity * 0.05)
}

fn freshness_score(row: &MemoryIndexRow, now_ms: f64) -> f64 {
    let stamp = row
        .source_created_at
        .as_ref()
        .or(row.last_accessed_at.as_ref())
        .unwrap_or(&row.indexed_at);
    let item_ms = js_sys::Date::parse(stamp);
    if !item_ms.is_finite() || !now_ms.is_finite() || now_ms <= item_ms {
        return 1.0;
    }
    let age_hours = (now_ms - item_ms) / 3_600_000.0;
    (-age_hours / 72.0).exp().clamp(0.0, 1.0)
}

fn estimate_tokens(title: &Option<String>, summary: &str, tags: &Option<String>) -> usize {
    let tag_chars = tags.as_ref().map(|t| t.len()).unwrap_or(0);
    (title.as_ref().map(|t| t.len()).unwrap_or(0) + summary.len() + tag_chars) / 4 + 16
}

fn tokenize(input: &str) -> HashSet<String> {
    input
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn latest_conflict_versions(rows: &[MemoryIndexRow]) -> HashMap<String, i64> {
    let mut latest: HashMap<String, i64> = HashMap::new();
    for row in rows {
        if let Some(key) = &row.conflict_key {
            let v = row.conflict_version.unwrap_or(1);
            latest
                .entry(key.clone())
                .and_modify(|cur: &mut i64| *cur = (*cur).max(v))
                .or_insert(v);
        }
    }
    latest
}

fn is_conflicted(row: &MemoryIndexRow, latest: &HashMap<String, i64>) -> bool {
    match &row.conflict_key {
        Some(k) => row.conflict_version.unwrap_or(1) < *latest.get(k).unwrap_or(&1),
        None => false,
    }
}

fn is_stale(row: &MemoryIndexRow, now: &str) -> bool {
    match &row.expires_at {
        Some(exp) => exp.as_str() <= now,
        None => false,
    }
}

fn parse_tags(tags: &Option<String>) -> Vec<String> {
    tags.as_ref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
}

async fn touch_memory_items(db: &D1Database, items: &[models::MemoryCandidate]) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let now = now_iso();
    let mut stmts = Vec::with_capacity(items.len());
    for item in items {
        let stmt = db
            .prepare(
                "UPDATE memory_index
                 SET access_count = access_count + 1, last_accessed_at = ?1
                 WHERE id = ?2",
            )
            .bind(&[JsValue::from_str(&now), JsValue::from_str(&item.id)])?;
        stmts.push(stmt);
    }
    db.batch(stmts).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn log_retrieval_query(
    db: &D1Database,
    query_id: &str,
    req: &models::RetrieveMemoryRequest,
    returned_count: i64,
    latency_ms: i64,
    stale_filtered: i64,
    unsafe_filtered: i64,
    conflict_filtered: i64,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO memory_retrieval_queries (
            id, repo, query_text, run_id, task_id, thread_id, top_k, related_repos,
            returned_count, latency_ms, stale_filtered, unsafe_filtered, conflict_filtered, created_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            ?9, ?10, ?11, ?12, ?13, ?14
        )",
    )
    .bind(&[
        JsValue::from_str(query_id),
        JsValue::from_str(&req.repo),
        JsValue::from_str(&req.query),
        opt_str(&req.run_id),
        opt_str(&req.task_id),
        opt_str(&req.thread_id),
        JsValue::from(req.top_k as f64),
        JsValue::from_str(&serde_json::to_string(&req.related_repos).unwrap_or_else(|_| "[]".to_string())),
        JsValue::from(returned_count),
        JsValue::from(latency_ms),
        JsValue::from(stale_filtered),
        JsValue::from(unsafe_filtered),
        JsValue::from(conflict_filtered),
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn list_policy_rules(db: &D1Database) -> Result<Vec<PolicyRuleRow>> {
    let result: D1Result = db
        .prepare("SELECT * FROM policy_rules ORDER BY priority DESC, created_at ASC")
        .bind(&[])?
        .all()
        .await?;
    result.results()
}

pub async fn get_policy_rule(db: &D1Database, id: &str) -> Result<Option<PolicyRuleRow>> {
    db.prepare("SELECT * FROM policy_rules WHERE id = ?1")
        .bind(&[JsValue::from_str(id)])?
        .first(None)
        .await
}

pub async fn update_policy_rule(
    db: &D1Database,
    id: &str,
    body: &models::UpdatePolicyRule,
) -> Result<bool> {
    let now = now_iso();
    let mut set_parts: Vec<String> = Vec::new();
    let mut bind_vals: Vec<JsValue> = Vec::new();
    let mut param_idx = 1u32;

    if let Some(ref v) = body.name {
        set_parts.push(format!("name = ?{param_idx}"));
        bind_vals.push(JsValue::from_str(v));
        param_idx += 1;
    }
    if let Some(ref v) = body.action_pattern {
        set_parts.push(format!("action_pattern = ?{param_idx}"));
        bind_vals.push(JsValue::from_str(v));
        param_idx += 1;
    }
    if let Some(ref v) = body.resource_pattern {
        set_parts.push(format!("resource_pattern = ?{param_idx}"));
        bind_vals.push(JsValue::from_str(v));
        param_idx += 1;
    }
    if let Some(ref v) = body.actor_pattern {
        set_parts.push(format!("actor_pattern = ?{param_idx}"));
        bind_vals.push(JsValue::from_str(v));
        param_idx += 1;
    }
    if let Some(ref v) = body.risk_level {
        set_parts.push(format!("risk_level = ?{param_idx}"));
        bind_vals.push(JsValue::from_str(v));
        param_idx += 1;
    }
    if let Some(ref v) = body.verdict {
        set_parts.push(format!("verdict = ?{param_idx}"));
        bind_vals.push(JsValue::from_str(v));
        param_idx += 1;
    }
    if let Some(ref v) = body.reason {
        set_parts.push(format!("reason = ?{param_idx}"));
        bind_vals.push(JsValue::from_str(v));
        param_idx += 1;
    }
    if let Some(v) = body.priority {
        set_parts.push(format!("priority = ?{param_idx}"));
        bind_vals.push(JsValue::from(v));
        param_idx += 1;
    }
    if let Some(v) = body.enabled {
        set_parts.push(format!("enabled = ?{param_idx}"));
        bind_vals.push(JsValue::from(if v { 1 } else { 0 }));
        param_idx += 1;
    }

    // Always update timestamp
    set_parts.push(format!("updated_at = ?{param_idx}"));
    bind_vals.push(JsValue::from_str(&now));
    param_idx += 1;

    // id goes last
    bind_vals.push(JsValue::from_str(id));

    let query = format!(
        "UPDATE policy_rules SET {} WHERE id = ?{param_idx}",
        set_parts.join(", ")
    );

    let res: D1Result = db.prepare(&query).bind(&bind_vals)?.run().await?;
    let changed = res
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);
    Ok(changed)
}

pub async fn delete_policy_rule(db: &D1Database, id: &str) -> Result<bool> {
    let res: D1Result = db
        .prepare("DELETE FROM policy_rules WHERE id = ?1")
        .bind(&[JsValue::from_str(id)])?
        .run()
        .await?;
    let changed = res
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);
    Ok(changed)
}

/// Evaluate a policy check against all enabled rules. Returns (verdict, reason, risk_level, matched_rule_id).
/// Rules are matched by specificity: exact match > prefix/pattern > wildcard.
/// If no rule matches, returns default-allow for reads and default-deny for destructive/irreversible.
pub async fn evaluate_policy(
    db: &D1Database,
    req: &models::PolicyCheckRequest,
) -> Result<(String, String, String, Option<String>)> {
    let result: D1Result = db
        .prepare(
            "SELECT * FROM policy_rules WHERE enabled = 1 ORDER BY priority DESC, created_at ASC",
        )
        .bind(&[])?
        .all()
        .await?;
    let rules: Vec<PolicyRuleRow> = result.results()?;

    let resource = req.resource.as_deref().unwrap_or("");

    // Find best matching rule by specificity score
    let mut best: Option<(&PolicyRuleRow, u32)> = None;
    for rule in &rules {
        if !pattern_matches(&rule.action_pattern, &req.action) {
            continue;
        }
        if !pattern_matches(&rule.resource_pattern, resource) {
            continue;
        }
        if !pattern_matches(&rule.actor_pattern, &req.actor) {
            continue;
        }
        let score = specificity_score(
            &rule.action_pattern,
            &rule.resource_pattern,
            &rule.actor_pattern,
        );
        if best.is_none() || score > best.unwrap().1 {
            best = Some((rule, score));
        }
    }

    match best {
        Some((rule, _)) => Ok((
            rule.verdict.clone(),
            rule.reason.clone(),
            rule.risk_level.clone(),
            Some(rule.id.clone()),
        )),
        None => {
            // Default policy: infer risk from action name
            let risk = classify_action_risk(&req.action);
            let (verdict, reason) = match risk.as_str() {
                "read" => ("allow", "no matching rule; read actions default-allowed"),
                "write" => ("allow", "no matching rule; write actions default-allowed"),
                "destructive" | "irreversible" => (
                    "escalate",
                    "no matching rule; high-risk action requires explicit policy",
                ),
                _ => ("allow", "no matching rule; default-allowed"),
            };
            Ok((verdict.into(), reason.into(), risk, None))
        }
    }
}

/// Simple pattern matching: '*' matches anything, 'prefix:*' matches prefix, exact otherwise.
fn pattern_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(':') {
        // "deploy:" matches "deploy:staging", "deploy:prod", etc.
        return value.starts_with(prefix) || value == prefix.trim_end_matches(':');
    }
    if let Some(prefix) = pattern.strip_suffix(":*") {
        return value.starts_with(&format!("{prefix}:")) || value == prefix;
    }
    pattern == value
}

/// Specificity: exact fields score higher than wildcards.
fn specificity_score(action: &str, resource: &str, actor: &str) -> u32 {
    let mut score = 0u32;
    if action != "*" {
        score += if action.contains('*') { 1 } else { 2 };
    }
    if resource != "*" {
        score += if resource.contains('*') { 1 } else { 2 };
    }
    if actor != "*" {
        score += if actor.contains('*') { 1 } else { 2 };
    }
    score
}

/// Classify action risk by naming convention.
fn classify_action_risk(action: &str) -> String {
    let a = action.to_lowercase();
    if a.starts_with("read")
        || a.starts_with("get")
        || a.starts_with("list")
        || a.starts_with("view")
        || a.starts_with("describe")
    {
        return "read".into();
    }
    if a.starts_with("delete")
        || a.starts_with("drop")
        || a.starts_with("destroy")
        || a.starts_with("purge")
    {
        return "destructive".into();
    }
    if a.starts_with("deploy:prod") || a.contains("irreversible") || a.starts_with("revoke") {
        return "irreversible".into();
    }
    "write".into()
}

pub async fn list_policy_decisions(
    db: &D1Database,
    action: Option<&str>,
    actor: Option<&str>,
    decision: Option<&str>,
    limit: u32,
) -> Result<Vec<PolicyDecisionRow>> {
    // Build dynamic WHERE
    let mut conditions: Vec<String> = Vec::new();
    let mut bindings: Vec<JsValue> = Vec::new();
    let mut idx = 1u32;

    if let Some(a) = action {
        conditions.push(format!("action = ?{idx}"));
        bindings.push(JsValue::from_str(a));
        idx += 1;
    }
    if let Some(a) = actor {
        conditions.push(format!("actor = ?{idx}"));
        bindings.push(JsValue::from_str(a));
        idx += 1;
    }
    if let Some(d) = decision {
        conditions.push(format!("decision = ?{idx}"));
        bindings.push(JsValue::from_str(d));
        idx += 1;
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let query = format!(
        "SELECT * FROM policy_decisions {where_clause} ORDER BY created_at DESC LIMIT ?{idx}"
    );
    bindings.push(JsValue::from(limit));

    let result: D1Result = db.prepare(&query).bind(&bindings)?.all().await?;
    result.results()
}

fn add_seconds_to_now(seconds: u64) -> String {
    let now = js_sys::Date::now();
    let future = js_sys::Date::new(&JsValue::from_f64(now + (seconds as f64 * 1000.0)));
    future.to_iso_string().as_string().unwrap()
}

fn random_hex_id() -> Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf)
        .map_err(|err| Error::RustError(format!("failed to generate id: {err}")))?;
    Ok(hex::encode(buf))
}

pub async fn check_and_increment_rate_limit(
    db: &D1Database,
    actor: &str,
    action_class: &str,
    window_seconds: i64,
    max_requests: i64,
) -> Result<bool> {
    let now = js_sys::Date::now() as i64 / 1000;
    let window_start = now - (now % window_seconds.max(1));
    let counter_id = format!("{actor}|{action_class}|{window_start}|{window_seconds}");
    let now_iso = now_iso();

    db.prepare(
        "INSERT INTO policy_rate_limit_counters (
            id, actor, action_class, window_start_epoch, window_seconds, count, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
         ON CONFLICT(id) DO UPDATE SET
            count = count + 1,
            updated_at = excluded.updated_at",
    )
    .bind(&[
        JsValue::from_str(&counter_id),
        JsValue::from_str(actor),
        JsValue::from_str(action_class),
        JsValue::from(window_start),
        JsValue::from(window_seconds),
        JsValue::from_str(&now_iso),
    ])?
    .run()
    .await?;

    let row: Option<RateCounterRow> = db
        .prepare("SELECT count FROM policy_rate_limit_counters WHERE id = ?1")
        .bind(&[JsValue::from_str(&counter_id)])?
        .first(None)
        .await?;

    let current = row.map(|r| r.count).unwrap_or(0);
    Ok(current > max_requests.max(1))
}

pub async fn run_retention_cleanup(
    db: &D1Database,
    bucket: &Bucket,
    req: &models::RetentionRunRequest,
) -> Result<models::RetentionRunResponse> {
    let events_cutoff = days_ago_expr(req.events_ttl_days.max(1));
    let policy_cutoff = days_ago_expr(req.policy_ttl_days.max(1));
    let checkpoints_cutoff = days_ago_expr(req.checkpoints_ttl_days.max(1));
    let artifacts_cutoff = days_ago_expr(req.artifacts_ttl_days.max(1));

    let events_deleted = delete_older_than(db, "events_bronze", "created_at", &events_cutoff).await?;
    let policy_decisions_deleted =
        delete_older_than(db, "policy_decisions", "created_at", &policy_cutoff).await?;

    let checkpoint_keys = list_old_keys(db, "checkpoints", "state_r2_key", "created_at", &checkpoints_cutoff).await?;
    let checkpoints_deleted = delete_older_than(db, "checkpoints", "created_at", &checkpoints_cutoff).await?;
    for key in checkpoint_keys {
        let _ = bucket.delete(&key).await;
    }

    let artifact_keys = list_old_keys(db, "artifacts", "key", "created_at", &artifacts_cutoff).await?;
    let artifacts_deleted = delete_older_than(db, "artifacts", "created_at", &artifacts_cutoff).await?;
    for key in artifact_keys {
        let _ = bucket.delete(&key).await;
    }

    Ok(models::RetentionRunResponse {
        events_deleted,
        artifacts_deleted,
        checkpoints_deleted,
        policy_decisions_deleted,
    })
}

async fn delete_older_than(
    db: &D1Database,
    table: &str,
    time_col: &str,
    cutoff: &str,
) -> Result<usize> {
    let count_sql = format!(
        "SELECT count(*) as count FROM {table} WHERE {time_col} < datetime('now', ?1)"
    );
    let count_row: Option<CountRow> = db
        .prepare(&count_sql)
        .bind(&[JsValue::from_str(cutoff)])?
        .first(None)
        .await?;
    let count = count_row.map(|r| r.count as usize).unwrap_or(0);

    let delete_sql = format!("DELETE FROM {table} WHERE {time_col} < datetime('now', ?1)");
    db.prepare(&delete_sql)
        .bind(&[JsValue::from_str(cutoff)])?
        .run()
        .await?;
    Ok(count)
}

async fn list_old_keys(
    db: &D1Database,
    table: &str,
    key_col: &str,
    time_col: &str,
    cutoff: &str,
) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT {key_col} as key FROM {table} WHERE {time_col} < datetime('now', ?1)"
    );
    let result: D1Result = db
        .prepare(&sql)
        .bind(&[JsValue::from_str(cutoff)])?
        .all()
        .await?;
    let rows: Vec<KeyRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.key).collect())
}

fn days_ago_expr(days: i64) -> String {
    format!("-{} days", days.max(1))
}

// ── Internal row types ──────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct TaskIdRow {
    id: String,
}

#[derive(Debug, serde::Deserialize)]
struct RetryRow {
    retry_count: i32,
    max_retries: i32,
}

#[derive(Debug, serde::Deserialize)]
struct CountRow {
    count: i64,
}

#[derive(Debug, serde::Deserialize)]
struct KeyRow {
    key: String,
}

#[derive(Debug, serde::Deserialize)]
struct RateCounterRow {
    count: i64,
}

#[derive(Debug, serde::Deserialize)]
pub struct TaskRow {
    pub id: String,
    pub job_id: String,
    pub task_type: String,
    pub priority: i32,
    pub status: String,
    pub params: Option<String>,
    pub result: Option<String>,
    pub agent_id: Option<String>,
    pub graph_ref: Option<String>,
    pub play_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub retry_count: i32,
    pub max_retries: i32,
    pub lease_expires_at: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

impl TaskRow {
    pub fn into_agent_task(self) -> models::AgentTask {
        models::AgentTask {
            id: self.id,
            job_id: self.job_id,
            task_type: self.task_type,
            priority: self.priority,
            status: self.status,
            params: self.params.and_then(|s| serde_json::from_str(&s).ok()),
            result: self.result.and_then(|s| serde_json::from_str(&s).ok()),
            agent_id: self.agent_id,
            graph_ref: self.graph_ref,
            play_id: self.play_id,
            parent_task_id: self.parent_task_id,
            retry_count: self.retry_count,
            max_retries: self.max_retries,
            lease_expires_at: self.lease_expires_at,
            created_at: self.created_at,
            completed_at: self.completed_at,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub capabilities: String,
    pub endpoint: Option<String>,
    pub last_heartbeat: Option<String>,
    pub status: String,
    pub metadata: Option<String>,
}

impl AgentRow {
    pub fn into_agent(self) -> models::Agent {
        models::Agent {
            id: self.id,
            name: self.name,
            capabilities: serde_json::from_str(&self.capabilities).unwrap_or_default(),
            endpoint: self.endpoint,
            last_heartbeat: self.last_heartbeat,
            status: self.status,
            metadata: self.metadata.and_then(|s| serde_json::from_str(&s).ok()),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct TraceEventRow {
    id: String,
    run_id: Option<String>,
    thread_id: Option<String>,
    event_type: String,
    node_id: Option<String>,
    actor: Option<String>,
    payload: Option<String>,
    created_at: String,
}

impl TraceEventRow {
    fn into_trace_event(self) -> models::TraceEvent {
        models::TraceEvent {
            id: self.id,
            run_id: self.run_id,
            thread_id: self.thread_id,
            event_type: self.event_type,
            node_id: self.node_id,
            actor: self.actor,
            payload: self.payload.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: self.created_at,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct CheckpointRow {
    pub id: String,
    pub thread_id: String,
    pub node_id: String,
    pub parent_id: Option<String>,
    pub state_r2_key: String,
    pub state_size_bytes: Option<i64>,
    pub metadata: Option<String>,
    pub created_at: String,
}

#[derive(Debug, serde::Deserialize)]
struct MemoryIdRow {
    id: String,
}

#[derive(Debug, serde::Deserialize)]
struct PercentileRow {
    latency_ms: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
struct MemoryEvalRow {
    total_queries: i64,
    cache_hit_rate: Option<f64>,
    success_rate: Option<f64>,
    first_pass_success_rate: Option<f64>,
}

#[derive(Debug, serde::Deserialize, Clone)]
struct MemoryIndexRow {
    id: String,
    repo: String,
    kind: String,
    run_id: Option<String>,
    task_id: Option<String>,
    thread_id: Option<String>,
    title: Option<String>,
    summary: String,
    tags: Option<String>,
    content_ref: Option<String>,
    success_rate: Option<f64>,
    source_created_at: Option<String>,
    indexed_at: String,
    last_accessed_at: Option<String>,
    access_count: i32,
    unsafe_reason: Option<String>,
    expires_at: Option<String>,
    conflict_key: Option<String>,
    conflict_version: Option<i64>,
}

impl CheckpointRow {
    pub fn into_checkpoint(self) -> models::Checkpoint {
        models::Checkpoint {
            id: self.id,
            thread_id: self.thread_id,
            node_id: self.node_id,
            parent_id: self.parent_id,
            state_r2_key: self.state_r2_key,
            state_size_bytes: self.state_size_bytes,
            metadata: self.metadata.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: self.created_at,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct MemoryRow {
    id: String,
    run_id: Option<String>,
    thread_id: String,
    scope: String,
    key: String,
    ref_type: String,
    ref_id: String,
    created_at: String,
    expires_at: Option<String>,
}

impl MemoryRow {
    fn into_memory(self) -> models::Memory {
        models::Memory {
            id: self.id,
            run_id: self.run_id,
            thread_id: self.thread_id,
            scope: self.scope,
            key: self.key,
            ref_type: self.ref_type,
            ref_id: self.ref_id,
            created_at: self.created_at,
            expires_at: self.expires_at,
        }
    }
}

// ── WS2 row types ───────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct RunRow {
    pub id: String,
    pub repo: String,
    pub status: String,
    pub trigger: Option<String>,
    pub actor: String,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<String>,
}

impl RunRow {
    pub fn into_run_response(self) -> RunResponse {
        RunResponse {
            id: self.id,
            repo: self.repo,
            status: self.status,
            trigger: self.trigger,
            actor: self.actor,
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata.and_then(|s| serde_json::from_str(&s).ok()),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct RunResponse {
    pub id: String,
    pub repo: String,
    pub status: String,
    pub trigger: Option<String>,
    pub actor: String,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Ws2TaskRow {
    pub id: String,
    pub run_id: String,
    pub plan_id: Option<String>,
    pub name: String,
    pub status: String,
    pub actor: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<String>,
}

impl Ws2TaskRow {
    pub fn into_task_response(self) -> Ws2TaskResponse {
        Ws2TaskResponse {
            id: self.id,
            run_id: self.run_id,
            plan_id: self.plan_id,
            name: self.name,
            status: self.status,
            actor: self.actor,
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata.and_then(|s| serde_json::from_str(&s).ok()),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct Ws2TaskResponse {
    pub id: String,
    pub run_id: String,
    pub plan_id: Option<String>,
    pub name: String,
    pub status: String,
    pub actor: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<serde_json::Value>,
}

// ── Provenance Links (WS3: causality chain) ─────────────────────

pub async fn insert_relationship(
    db: &D1Database,
    rel_type: &str,
    from_kind: &str,
    from_id: &str,
    to_kind: &str,
    to_id: &str,
    relation: Option<&str>,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT OR IGNORE INTO relationships (rel_type, from_kind, from_id, to_kind, to_id, relation, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(&[
        JsValue::from_str(rel_type),
        JsValue::from_str(from_kind),
        JsValue::from_str(from_id),
        JsValue::from_str(to_kind),
        JsValue::from_str(to_id),
        match relation {
            Some(r) => JsValue::from_str(r),
            None => JsValue::NULL,
        },
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;
    Ok(())
}

/// Walk the provenance chain from an entity. Direction "forward" follows from→to; "backward" follows to→from.
pub async fn get_provenance_chain(
    db: &D1Database,
    kind: &str,
    id: &str,
    direction: &str,
    max_hops: u32,
) -> Result<Vec<models::ProvenanceEdge>> {
    let query = if direction == "backward" {
        "WITH RECURSIVE chain(depth, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at) AS (
           SELECT 0, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at
             FROM relationships WHERE to_kind = ?1 AND to_id = ?2
           UNION
           SELECT c.depth + 1, r.rel_type, r.from_kind, r.from_id, r.to_kind, r.to_id, r.relation, r.created_at
             FROM relationships r JOIN chain c ON r.to_kind = c.from_kind AND r.to_id = c.from_id
             WHERE c.depth < ?3
         ) SELECT * FROM chain ORDER BY depth"
    } else {
        "WITH RECURSIVE chain(depth, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at) AS (
           SELECT 0, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at
             FROM relationships WHERE from_kind = ?1 AND from_id = ?2
           UNION
           SELECT c.depth + 1, r.rel_type, r.from_kind, r.from_id, r.to_kind, r.to_id, r.relation, r.created_at
             FROM relationships r JOIN chain c ON r.from_kind = c.to_kind AND r.from_id = c.to_id
             WHERE c.depth < ?3
         ) SELECT * FROM chain ORDER BY depth"
    };

    let result: D1Result = db
        .prepare(query)
        .bind(&[
            JsValue::from_str(kind),
            JsValue::from_str(id),
            JsValue::from(max_hops),
        ])?
        .all()
        .await?;

    let rows: Vec<ProvenanceEdgeRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_edge()).collect())
}

/// Extract implicit causality edges from a GraphEvent and insert them.
pub async fn insert_causality_from_event(db: &D1Database, evt: &models::GraphEvent) -> Result<()> {
    // run → task causality
    if let (Some(ref run_id), Some(ref node_id)) = (&evt.run_id, &evt.node_id) {
        insert_relationship(
            db,
            "causality",
            "run",
            run_id,
            "node",
            node_id,
            Some("executed"),
        )
        .await?;
    }
    Ok(())
}

// ── Gold Layer: Run Summaries (WS3) ─────────────────────────────

pub async fn upsert_run_summary(
    db: &D1Database,
    run_id: &str,
    actor: Option<&str>,
    event_type: &str,
    created_at: &str,
) -> Result<()> {
    let now = now_iso();
    let actor_json: Vec<&str> = actor.into_iter().collect();
    let event_type_json = vec![event_type];

    // Atomic upsert: INSERT on first event, ON CONFLICT merge arrays + update counts.
    // json_group_array(DISTINCT ...) via subquery deduplicates actors/event_types.
    db.prepare(
        "INSERT INTO run_summaries (run_id, event_count, first_event_at, last_event_at, actors, event_types, updated_at)
         VALUES (?1, 1, ?2, ?2, ?3, ?4, ?5)
         ON CONFLICT(run_id) DO UPDATE SET
           event_count = event_count + 1,
           first_event_at = MIN(first_event_at, excluded.first_event_at),
           last_event_at = MAX(last_event_at, excluded.last_event_at),
           actors = (
             SELECT json_group_array(value) FROM (
               SELECT DISTINCT value FROM (
                 SELECT value FROM json_each(run_summaries.actors)
                 UNION ALL
                 SELECT value FROM json_each(excluded.actors)
               )
             )
           ),
           event_types = (
             SELECT json_group_array(value) FROM (
               SELECT DISTINCT value FROM (
                 SELECT value FROM json_each(run_summaries.event_types)
                 UNION ALL
                 SELECT value FROM json_each(excluded.event_types)
               )
             )
           ),
           updated_at = excluded.updated_at",
    )
    .bind(&[
        JsValue::from_str(run_id),
        JsValue::from_str(created_at),
        JsValue::from_str(&serde_json::to_string(&actor_json).unwrap()),
        JsValue::from_str(&serde_json::to_string(&event_type_json).unwrap()),
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn get_run_summary(db: &D1Database, run_id: &str) -> Result<Option<models::RunSummary>> {
    let row: Option<RunSummaryRow> = db
        .prepare("SELECT * FROM run_summaries WHERE run_id = ?1")
        .bind(&[JsValue::from_str(run_id)])?
        .first(None)
        .await?;
    Ok(row.map(|r| r.into_summary()))
}

pub async fn list_run_summaries(db: &D1Database, limit: u32) -> Result<Vec<models::RunSummary>> {
    let result: D1Result = db
        .prepare("SELECT * FROM run_summaries ORDER BY updated_at DESC LIMIT ?1")
        .bind(&[JsValue::from(limit)])?
        .all()
        .await?;
    let rows: Vec<RunSummaryRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_summary()).collect())
}

#[derive(Debug, serde::Deserialize)]
struct ProvenanceEdgeRow {
    depth: i32,
    rel_type: String,
    from_kind: String,
    from_id: String,
    to_kind: String,
    to_id: String,
    relation: Option<String>,
    created_at: Option<String>,
}

impl ProvenanceEdgeRow {
    fn into_edge(self) -> models::ProvenanceEdge {
        models::ProvenanceEdge {
            depth: self.depth,
            rel_type: self.rel_type,
            from_kind: self.from_kind,
            from_id: self.from_id,
            to_kind: self.to_kind,
            to_id: self.to_id,
            relation: self.relation,
            created_at: self.created_at,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct RunSummaryRow {
    run_id: String,
    event_count: i32,
    first_event_at: Option<String>,
    last_event_at: Option<String>,
    actors: Option<String>,
    event_types: Option<String>,
    updated_at: String,
}

impl RunSummaryRow {
    fn into_summary(self) -> models::RunSummary {
        models::RunSummary {
            run_id: self.run_id,
            event_count: self.event_count,
            first_event_at: self.first_event_at,
            last_event_at: self.last_event_at,
            actors: self
                .actors
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default(),
            event_types: self
                .event_types
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default(),
            updated_at: self.updated_at,
        }
    }
}

// ── WS4 Policy row types ────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
pub struct PolicyRuleRow {
    pub id: String,
    pub name: String,
    pub action_pattern: String,
    pub resource_pattern: String,
    pub actor_pattern: String,
    pub risk_level: String,
    pub verdict: String,
    pub reason: String,
    pub priority: i32,
    pub enabled: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl PolicyRuleRow {
    pub fn into_response(self) -> models::PolicyRuleResponse {
        models::PolicyRuleResponse {
            id: self.id,
            name: self.name,
            action_pattern: self.action_pattern,
            resource_pattern: self.resource_pattern,
            actor_pattern: self.actor_pattern,
            risk_level: self.risk_level,
            verdict: self.verdict,
            reason: self.reason,
            priority: self.priority,
            enabled: self.enabled != 0,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct PolicyDecisionRow {
    pub id: String,
    pub action: String,
    pub actor: String,
    pub resource: Option<String>,
    pub decision: String,
    pub reason: String,
    pub created_at: String,
    pub context: Option<String>,
}

impl PolicyDecisionRow {
    pub fn into_response(self) -> models::PolicyDecisionResponse {
        models::PolicyDecisionResponse {
            id: self.id,
            action: self.action,
            actor: self.actor,
            resource: self.resource,
            decision: self.decision,
            reason: self.reason,
            created_at: self.created_at,
            context: self.context.and_then(|s| serde_json::from_str(&s).ok()),
        }
    }
}

fn lease_time(seconds: u64) -> String {
    let now = js_sys::Date::now();
    let future = js_sys::Date::new(&JsValue::from_f64(now + (seconds as f64 * 1000.0)));
    future.to_iso_string().as_string().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Pattern matching ────────────────────────────────────────

    #[test]
    fn wildcard_matches_anything() {
        assert!(pattern_matches("*", "deploy"));
        assert!(pattern_matches("*", ""));
        assert!(pattern_matches("*", "anything:at:all"));
    }

    #[test]
    fn exact_match() {
        assert!(pattern_matches("deploy", "deploy"));
        assert!(!pattern_matches("deploy", "delete"));
        assert!(!pattern_matches("deploy", "deploy:staging"));
    }

    #[test]
    fn prefix_wildcard_match() {
        assert!(pattern_matches("deploy:*", "deploy:staging"));
        assert!(pattern_matches("deploy:*", "deploy:prod"));
        assert!(pattern_matches("deploy:*", "deploy"));
        assert!(!pattern_matches("deploy:*", "delete:staging"));
    }

    // ── Specificity scoring ─────────────────────────────────────

    #[test]
    fn all_wildcards_score_zero() {
        assert_eq!(specificity_score("*", "*", "*"), 0);
    }

    #[test]
    fn exact_fields_score_higher() {
        assert!(specificity_score("deploy", "*", "*") > specificity_score("*", "*", "*"));
        assert!(specificity_score("deploy", "prod", "*") > specificity_score("deploy", "*", "*"));
        assert!(
            specificity_score("deploy", "prod", "agent-1")
                > specificity_score("deploy", "prod", "*")
        );
    }

    #[test]
    fn prefix_scores_between_wildcard_and_exact() {
        let prefix = specificity_score("deploy:*", "*", "*");
        let exact = specificity_score("deploy", "*", "*");
        let wildcard = specificity_score("*", "*", "*");
        assert!(prefix > wildcard);
        assert!(exact > prefix);
    }

    // ── Risk classification ─────────────────────────────────────

    #[test]
    fn read_actions_classified() {
        assert_eq!(classify_action_risk("read:config"), "read");
        assert_eq!(classify_action_risk("get:status"), "read");
        assert_eq!(classify_action_risk("list:agents"), "read");
        assert_eq!(classify_action_risk("view:logs"), "read");
        assert_eq!(classify_action_risk("describe:resources"), "read");
    }

    #[test]
    fn destructive_actions_classified() {
        assert_eq!(classify_action_risk("delete:resource"), "destructive");
        assert_eq!(classify_action_risk("drop:table"), "destructive");
        assert_eq!(classify_action_risk("destroy:cluster"), "destructive");
        assert_eq!(classify_action_risk("purge:cache"), "destructive");
    }

    #[test]
    fn irreversible_actions_classified() {
        assert_eq!(classify_action_risk("deploy:prod"), "irreversible");
        assert_eq!(classify_action_risk("revoke:token"), "irreversible");
    }

    #[test]
    fn write_is_default() {
        assert_eq!(classify_action_risk("update:config"), "write");
        assert_eq!(classify_action_risk("create:resource"), "write");
        assert_eq!(classify_action_risk("deploy:staging"), "write");
    }

    #[test]
    fn tokenization_works() {
        let t = tokenize("Fix CI checks for PR-31");
        assert!(t.contains("fix"));
        assert!(t.contains("checks"));
        assert!(t.contains("pr"));
    }

    #[test]
    fn jaccard_scoring_is_monotonic() {
        let a = tokenize("graph event replay");
        let b = tokenize("graph event replay state");
        let c = tokenize("unrelated topic");
        assert!(jaccard_similarity(&a, &b) > jaccard_similarity(&a, &c));
    }
}
