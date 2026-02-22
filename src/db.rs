use crate::models;
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
        "INSERT INTO tasks (id, job_id, task_type, priority, status, params, graph_ref, play_id, parent_task_id, max_retries, created_at)
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

    // Capability filter with parameterized placeholders (no SQL injection)
    let (where_caps, bind_caps): (String, Vec<JsValue>) = if capabilities.is_empty() {
        ("1=1".to_string(), vec![])
    } else {
        let placeholders = (1..=capabilities.len())
            .map(|i| format!("task_type = ?{i}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let values = capabilities
            .iter()
            .map(|c| JsValue::from_str(c))
            .collect::<Vec<_>>();
        (placeholders, values)
    };

    let query = format!(
        "SELECT id FROM tasks WHERE status = 'pending' AND ({where_caps}) ORDER BY priority DESC, created_at ASC LIMIT 1"
    );

    let result: Option<TaskIdRow> = db.prepare(&query).bind(&bind_caps)?.first(None).await?;

    let task_id = match result {
        Some(row) => row.id,
        None => return Ok(None),
    };

    // Claim it atomically: only succeeds if still pending
    db.prepare(
        "UPDATE tasks SET status = 'running', agent_id = ?1, lease_expires_at = ?2 WHERE id = ?3 AND status = 'pending'",
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
        .prepare("SELECT * FROM tasks WHERE id = ?1 AND agent_id = ?2")
        .bind(&[JsValue::from_str(&task_id), JsValue::from_str(agent_id)])?
        .first(None)
        .await?;

    Ok(task.map(|r| r.into_agent_task()))
}

pub async fn heartbeat_task(db: &D1Database, task_id: &str, agent_id: &str) -> Result<bool> {
    let lease = lease_time(300);
    let result: D1Result = db
        .prepare(
            "UPDATE tasks SET lease_expires_at = ?1 WHERE id = ?2 AND agent_id = ?3 AND status = 'running'",
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
            "UPDATE tasks SET status = 'completed', result = ?1, completed_at = ?2 WHERE id = ?3 AND status = 'running'",
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
        .prepare("SELECT retry_count, max_retries FROM tasks WHERE id = ?1 AND status = 'running'")
        .bind(&[JsValue::from_str(task_id)])?
        .first(None)
        .await?;

    let (new_status, new_retry) = match task {
        Some(row) if row.retry_count < row.max_retries => ("pending", row.retry_count + 1),
        _ => ("failed", 0),
    };

    let result_json = serde_json::json!({ "error": error }).to_string();

    db.prepare(
        "UPDATE tasks SET status = ?1, retry_count = CASE WHEN ?1 = 'pending' THEN ?2 ELSE retry_count END, result = ?3, agent_id = CASE WHEN ?1 = 'pending' THEN NULL ELSE agent_id END, lease_expires_at = NULL, completed_at = CASE WHEN ?1 = 'failed' THEN ?4 ELSE NULL END WHERE id = ?5",
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

fn lease_time(seconds: u64) -> String {
    let now = js_sys::Date::now();
    let future = js_sys::Date::new(&JsValue::from_f64(now + (seconds as f64 * 1000.0)));
    future.to_iso_string().as_string().unwrap()
}
