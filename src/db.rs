use crate::models;
use wasm_bindgen::JsValue;
use worker::*;

pub fn now_iso() -> String {
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

// ── WS2 Domain: Policy Decisions ────────────────────────────────

pub async fn record_policy_check(
    db: &D1Database,
    id: &str,
    body: &models::PolicyCheckRequest,
    decision: &str,
    reason: &str,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO policy_decisions (id, action, actor, resource, decision, reason, created_at, context)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(&body.action),
        JsValue::from_str(&body.actor),
        opt_str(&body.resource),
        JsValue::from_str(decision),
        JsValue::from_str(reason),
        JsValue::from_str(&now),
        opt_json(&body.context),
    ])?
    .run()
    .await?;
    Ok(())
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

fn lease_time(seconds: u64) -> String {
    let now = js_sys::Date::now();
    let future = js_sys::Date::new(&JsValue::from_f64(now + (seconds as f64 * 1000.0)));
    future.to_iso_string().as_string().unwrap()
}

// ── Integrations (WS6) ─────────────────────────────────────────

pub async fn register_integration(
    db: &D1Database,
    id: &str,
    body: &crate::integrations::RegisterIntegration,
) -> Result<()> {
    let now = now_iso();
    let config_json = match &body.config {
        Some(c) => Some(
            serde_json::to_string(c)
                .map_err(|e| Error::RustError(format!("config serialization: {e}")))?,
        ),
        None => None,
    };
    let api_version = body.api_version.as_deref().unwrap_or("v1");

    db.prepare(
        "INSERT INTO integrations (id, target, name, endpoint, api_version, status, config, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7)
         ON CONFLICT(target, name) DO UPDATE SET
           endpoint = excluded.endpoint,
           api_version = excluded.api_version,
           config = excluded.config,
           status = 'active',
           updated_at = excluded.created_at",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(body.target.as_str()),
        JsValue::from_str(&body.name),
        opt_str(&body.endpoint),
        JsValue::from_str(api_version),
        match &config_json { Some(s) => JsValue::from_str(s), None => JsValue::NULL },
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;

    Ok(())
}

pub async fn list_integrations(db: &D1Database, limit: u32) -> Result<Vec<serde_json::Value>> {
    let result: D1Result = db
        .prepare(
            "SELECT * FROM integrations WHERE status = 'active' ORDER BY created_at DESC LIMIT ?1",
        )
        .bind(&[JsValue::from(limit)])?
        .all()
        .await?;

    let rows: Vec<serde_json::Value> = result.results()?;
    Ok(rows)
}

pub async fn touch_integration(db: &D1Database, target: &str) -> Result<()> {
    let now = now_iso();
    db.prepare("UPDATE integrations SET last_seen_at = ?1 WHERE target = ?2 AND status = 'active'")
        .bind(&[JsValue::from_str(&now), JsValue::from_str(target)])?
        .run()
        .await?;
    Ok(())
}
