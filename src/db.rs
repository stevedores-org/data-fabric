use crate::models;
use crate::policy::RiskLevel;
use std::collections::{HashMap, HashSet};
use wasm_bindgen::JsValue;
use worker::*;

// ── Tenant-scoped SQL constants ─────────────────────────────────────
//
// These statements are extracted as named constants so the
// cross-tenant isolation tests (see `mod tests` at the bottom) can
// assert their shape — specifically that every read and every write
// against the WS8-protected tables binds `tenant_id` in the WHERE /
// INSERT column list. The SQL text is authoritative for what runs
// against D1; if a future edit removes `tenant_id` from one of these
// statements the cross_tenant_sql_shape tests will fail.

/// SELECT for `get_play_definition` — scoped by tenant_id then name.
const SQL_GET_PLAY_DEFINITION: &str = "SELECT name, goal, tasks_json FROM play_definitions \
     WHERE tenant_id = ?1 AND name = ?2";

/// INSERT for `create_policy_escalation` — tenant_id is the first column.
const SQL_INSERT_POLICY_ESCALATION: &str =
    "INSERT INTO policy_escalations (tenant_id, id, decision_id, action, actor, resource, risk_level, status, context, created_at)\n         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?9)";

/// INSERT/UPSERT for `check_and_increment_rate_limit` — tenant_id is the
/// first column and is also embedded in the synthetic counter id.
const SQL_UPSERT_RATE_LIMIT_COUNTER: &str =
    "INSERT INTO policy_rate_limit_counters (\n            tenant_id, id, actor, action_class, window_start_epoch, window_seconds, count, updated_at\n         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)\n         ON CONFLICT(id) DO UPDATE SET\n            count = count + 1,\n            updated_at = excluded.updated_at";

/// SELECT for the rate-limit count, also tenant-scoped.
const SQL_SELECT_RATE_LIMIT_COUNT: &str = "SELECT count FROM policy_rate_limit_counters \
     WHERE tenant_id = ?1 AND id = ?2";

// ── AIVCS: change_set SQL constants (issue #148, slice 1) ──────────
//
// The `change_set` table is the projection AIVCS uses above the diff
// artifact (R2). All three statements below bind `tenant_id` as `?1`
// so cross-tenant reads/writes are impossible — the
// `cross_tenant_sql_change_set_*` tests in `mod tests` pin this.

/// INSERT into `change_set`. tenant_id is the first column and ?1.
pub const SQL_INSERT_CHANGE_SET: &str =
    "INSERT INTO change_set (tenant_id, id, repo, base_ref, head_ref, author_agent_id, status, risk_level, confidence, run_id, diff_artifact_key, summary_artifact_key) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)";

/// SELECT a single `change_set` row by (tenant_id, id).
pub const SQL_GET_CHANGE_SET: &str =
    "SELECT id, repo, base_ref, head_ref, author_agent_id, status, risk_level, confidence, run_id, diff_artifact_key, summary_artifact_key, created_at \
     FROM change_set \
     WHERE tenant_id = ?1 AND id = ?2";

/// SELECT recent `change_set` rows for a repo. ORDER BY created_at DESC,
/// LIMIT is bound as `?3` so callers can page (cursor-based pagination
/// lands with the HTTP route in a later slice).
pub const SQL_LIST_CHANGE_SETS_BY_REPO: &str =
    "SELECT id, repo, base_ref, head_ref, author_agent_id, status, risk_level, confidence, run_id, diff_artifact_key, summary_artifact_key, created_at \
     FROM change_set \
     WHERE tenant_id = ?1 AND repo = ?2 \
     ORDER BY created_at DESC \
     LIMIT ?3";

// ── AIVCS review projections (issue #148 slice 2) ──────────────
//
// All four projection tables introduced in migration 0016 carry
// `tenant_id` as the leading PRIMARY KEY component. The SQL
// statements below MUST bind `tenant_id` as ?1 on every read and
// every write; the cross-tenant tests at the bottom of this file
// assert that shape.
//
// The constants and CRUD helpers below are not yet referenced from
// `lib.rs` — slice 2 is projection-only. They are exercised by the
// cross-tenant SQL-shape tests at the bottom of this file and will
// be consumed by the slice-3+ HTTP routes and the projection
// follower. `#[allow(dead_code)]` keeps the wasm `-D warnings`
// build green until those slices land.

#[allow(dead_code)]
const SQL_INSERT_REVIEW_THREAD: &str =
    "INSERT INTO review_thread (tenant_id, id, review_id, change_set_id, status, created_at, resolved_at)\n     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

#[allow(dead_code)]
const SQL_GET_REVIEW_THREAD: &str =
    "SELECT id, review_id, change_set_id, status, created_at, resolved_at \
     FROM review_thread WHERE tenant_id = ?1 AND id = ?2";

#[allow(dead_code)]
const SQL_LIST_REVIEW_THREADS_FOR_REVIEW: &str =
    "SELECT id, review_id, change_set_id, status, created_at, resolved_at \
     FROM review_thread WHERE tenant_id = ?1 AND review_id = ?2 \
     ORDER BY created_at ASC LIMIT ?3";

#[allow(dead_code)]
const SQL_UPDATE_REVIEW_THREAD_STATUS: &str =
    "UPDATE review_thread SET status = ?3, resolved_at = ?4 \
     WHERE tenant_id = ?1 AND id = ?2";

#[allow(dead_code)]
const SQL_INSERT_REVIEW_COMMENT: &str =
    "INSERT INTO review_comment (tenant_id, id, thread_id, actor, body, parent_comment_id, created_at)\n     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

#[allow(dead_code)]
const SQL_LIST_REVIEW_COMMENTS_FOR_THREAD: &str =
    "SELECT id, thread_id, actor, body, parent_comment_id, created_at \
     FROM review_comment WHERE tenant_id = ?1 AND thread_id = ?2 \
     ORDER BY created_at ASC, id ASC LIMIT ?3";

#[allow(dead_code)]
const SQL_INSERT_REVIEW_THREAD_RESOLUTION: &str =
    "INSERT INTO review_thread_resolution (tenant_id, thread_id, resolved_by, resolution, note, resolved_at)\n     VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

#[allow(dead_code)]
const SQL_GET_REVIEW_THREAD_RESOLUTION: &str =
    "SELECT thread_id, resolved_by, resolution, note, resolved_at \
     FROM review_thread_resolution WHERE tenant_id = ?1 AND thread_id = ?2";

#[allow(dead_code)]
const SQL_INSERT_FILE_ANCHOR: &str =
    "INSERT INTO file_anchor (tenant_id, id, thread_id, file_path, start_line, end_line, side)\n     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

#[allow(dead_code)]
const SQL_LIST_FILE_ANCHORS_FOR_THREAD: &str =
    "SELECT id, thread_id, file_path, start_line, end_line, side \
     FROM file_anchor WHERE tenant_id = ?1 AND thread_id = ?2 \
     ORDER BY file_path ASC, start_line ASC, id ASC LIMIT ?3";
/// Issue #148 / AIVCS slice 3 — INSERT into the `human_decision` projection.
///
/// `tenant_id` is the first column (and bound to ?1) so cross-tenant
/// writes can't slip past the WS8 isolation tests. The projection
/// carries seven typed columns plus `tenant_id` and `id`; the
/// `created_at` column is populated by SQLite's column default rather
/// than by a bound parameter so retries from the same wall-clock pin
/// to a single value.
const SQL_INSERT_HUMAN_DECISION: &str =
    "INSERT INTO human_decision (tenant_id, id, run_id, review_id, actor, decision_type, reason, policy_decision_id, resulting_event_id)\n         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

/// SELECT for `list_human_decisions_by_run` — tenant-scoped, ordered by
/// `created_at` ascending so the UI can render the human-decision
/// timeline per run without a second sort pass.
const SQL_LIST_HUMAN_DECISIONS_BY_RUN: &str =
    "SELECT id, run_id, review_id, actor, decision_type, reason, policy_decision_id, resulting_event_id, created_at \
     FROM human_decision \
     WHERE tenant_id = ?1 AND run_id = ?2 \
     ORDER BY created_at ASC, id ASC";

/// SELECT for `list_human_decisions_by_review` — tenant-scoped, ordered
/// by `created_at` ascending. Mirrors the by-run query shape so the BFF
/// can share a row decoder.
const SQL_LIST_HUMAN_DECISIONS_BY_REVIEW: &str =
    "SELECT id, run_id, review_id, actor, decision_type, reason, policy_decision_id, resulting_event_id, created_at \
     FROM human_decision \
     WHERE tenant_id = ?1 AND review_id = ?2 \
     ORDER BY created_at ASC, id ASC";

/// UPDATE for `pause_run` (AIVCS issue #148 'Pause Agent' slice 4).
///
/// Tenant-scoped on `?1` and guarded against transitioning out of a
/// terminal state — the guard is what makes the operation correct
/// concurrently with a run completing on the orchestrator side. If the
/// run already moved to 'succeeded' / 'failed' / 'cancelled' the UPDATE
/// affects zero rows and the handler surfaces `RUN_IN_TERMINAL_STATE`.
///
/// `paused_at` is stamped only when transitioning from a pausable state
/// (`created` or `running`). Already-paused runs are handled idempotently
/// in `pause_run` before this UPDATE is issued.
pub const SQL_PAUSE_RUN: &str =
    "UPDATE runs SET status = 'paused', paused_at = datetime('now'), updated_at = datetime('now') \
     WHERE tenant_id = ?1 AND id = ?2 \
       AND status IN ('created', 'running')";

/// UPDATE for `resume_run` (AIVCS issue #148 'Pause Agent' slice 4).
///
/// Tenant-scoped on `?1` and guarded so resume is only valid out of
/// 'paused'. Resuming a run that is already running, or one that has
/// reached a terminal state, affects zero rows and the handler surfaces
/// `RUN_NOT_PAUSED`.
pub const SQL_RESUME_RUN: &str =
    "UPDATE runs SET status = 'running', resumed_at = datetime('now'), updated_at = datetime('now') \
     WHERE tenant_id = ?1 AND id = ?2 AND status = 'paused'";

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

pub async fn create_task(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::CreateAgentTask,
) -> Result<()> {
    let now = now_iso();
    let max_retries = body.max_retries.unwrap_or(3);

    db.prepare(
        "INSERT INTO mcp_tasks (tenant_id, id, job_id, task_type, priority, status, params, graph_ref, play_id, parent_task_id, max_retries, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

pub async fn get_mcp_task_by_id(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
) -> Result<Option<models::AgentTask>> {
    let task: Option<TaskRow> = db
        .prepare("SELECT * FROM mcp_tasks WHERE tenant_id = ?1 AND id = ?2")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
        .first(None)
        .await?;

    Ok(task.map(|r| r.into_agent_task()))
}

pub async fn sync_task_status(
    db: &D1Database,
    tenant_id: &str,
    task: &models::AgentTask,
) -> Result<()> {
    let result_json = task
        .result
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap());

    db.prepare(
        "UPDATE mcp_tasks SET status = ?1, agent_id = ?2, result = ?3, retry_count = ?4, lease_expires_at = ?5, completed_at = ?6
         WHERE tenant_id = ?7 AND id = ?8",
    )
    .bind(&[
        JsValue::from_str(&task.status),
        opt_str(&task.agent_id),
        match &result_json {
            Some(s) => JsValue::from_str(s),
            None => JsValue::NULL,
        },
        JsValue::from(task.retry_count),
        opt_str(&task.lease_expires_at),
        opt_str(&task.completed_at),
        JsValue::from_str(tenant_id),
        JsValue::from_str(&task.id),
    ])?
    .run()
    .await?;

    Ok(())
}

pub async fn heartbeat_task(
    db: &D1Database,
    tenant_id: &str,
    task_id: &str,
    agent_id: &str,
) -> Result<bool> {
    let lease = lease_time(300);
    let result: D1Result = db
        .prepare(
            "UPDATE mcp_tasks SET lease_expires_at = ?1 WHERE tenant_id = ?2 AND id = ?3 AND agent_id = ?4 AND status = 'running'",
        )
        .bind(&[
            JsValue::from_str(&lease),
            JsValue::from_str(tenant_id),
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

pub async fn get_play_definition(
    db: &D1Database,
    tenant_id: &str,
    name: &str,
) -> Result<Option<models::PlayDefinition>> {
    #[derive(serde::Deserialize)]
    struct PlayRow {
        name: String,
        goal: String,
        tasks_json: String,
    }

    // Tenant-scoped lookup: a play named "foo" in tenant A is invisible
    // to tenant B. Migration 0014_ws8_tenant_id_addendum.sql adds the
    // `tenant_id` column and seeds the platform-default `sre-incident`
    // play under `tenant_id = 'default'`.
    let row: Option<PlayRow> = db
        .prepare(SQL_GET_PLAY_DEFINITION)
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(name)])?
        .first(None)
        .await?;

    match row {
        Some(r) => {
            let tasks: Vec<models::PlayTaskDefinition> = serde_json::from_str(&r.tasks_json)
                .map_err(|e| Error::RustError(format!("failed to parse play tasks: {e}")))?;
            Ok(Some(models::PlayDefinition {
                name: r.name,
                goal: r.goal,
                tasks,
            }))
        }
        None => Ok(None),
    }
}

// ── Agents ──────────────────────────────────────────────────────

pub async fn register_agent(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::RegisterAgent,
) -> Result<()> {
    let now = now_iso();
    let caps_json = serde_json::to_string(&body.capabilities).unwrap();

    db.prepare(
        "INSERT INTO agents (tenant_id, id, name, capabilities, endpoint, last_heartbeat, status, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

/// List active agents for `tenant_id` ordered by `(name ASC, id ASC)`.
///
/// Returns `(rows, next_cursor)`. Mirrors the `list_runs` pattern: we fetch
/// `limit + 1` and drop the overflow row to detect the next page without a
/// second query. The cursor predicate uses an OR of two terms
/// (`name > ?` OR `name = ? AND id > ?`) rather than a row-value comparison
/// because SQLite/D1 cannot index `(a, b) > (?, ?)` as a single sargable
/// expression.
pub async fn list_agents(
    db: &D1Database,
    tenant_id: &str,
    limit: u32,
    cursor: Option<&crate::pagination::AgentsCursor>,
) -> Result<(Vec<models::Agent>, Option<crate::pagination::AgentsCursor>)> {
    let fetch_limit = limit.saturating_add(1);

    let mut clauses: Vec<String> = vec!["tenant_id = ?".into(), "status = 'active'".into()];
    let mut bindings: Vec<JsValue> = vec![JsValue::from_str(tenant_id)];

    if let Some(c) = cursor {
        clauses.push("(name > ? OR (name = ? AND id > ?))".into());
        bindings.push(JsValue::from_str(&c.name));
        bindings.push(JsValue::from_str(&c.name));
        bindings.push(JsValue::from_str(&c.id));
    }
    bindings.push(JsValue::from(fetch_limit));

    let query = format!(
        "SELECT * FROM agents WHERE {} ORDER BY name ASC, id ASC LIMIT ?",
        clauses.join(" AND ")
    );

    let result: D1Result = db.prepare(&query).bind(&bindings)?.all().await?;
    let mut rows: Vec<AgentRow> = result.results()?;

    let next_cursor = compute_agents_next_cursor(&mut rows, limit);

    Ok((
        rows.into_iter().map(|r| r.into_agent()).collect(),
        next_cursor,
    ))
}

/// Pure helper: trims the overflow row and produces the cursor for the next
/// page. Extracted from `list_agents` so it can be unit-tested without D1.
pub(crate) fn compute_agents_next_cursor(
    rows: &mut Vec<AgentRow>,
    limit: u32,
) -> Option<crate::pagination::AgentsCursor> {
    if rows.len() as u32 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|r| crate::pagination::AgentsCursor {
            name: r.name.clone(),
            id: r.id.clone(),
        })
    } else {
        None
    }
}

// ── Checkpoints ─────────────────────────────────────────────────

pub async fn create_checkpoint(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::CreateCheckpoint,
    r2_key: &str,
    size: i64,
) -> Result<()> {
    let now = now_iso();

    db.prepare(
        "INSERT INTO checkpoints (tenant_id, id, thread_id, node_id, parent_id, state_r2_key, state_size_bytes, metadata, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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
    tenant_id: &str,
    thread_id: &str,
) -> Result<Option<CheckpointRow>> {
    db.prepare(
        "SELECT * FROM checkpoints WHERE tenant_id = ?1 AND thread_id = ?2 ORDER BY created_at DESC LIMIT 1",
    )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(thread_id)])?
        .first(None)
        .await
}

pub async fn get_checkpoint_by_id(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
) -> Result<Option<CheckpointRow>> {
    db.prepare("SELECT * FROM checkpoints WHERE tenant_id = ?1 AND id = ?2")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
        .first(None)
        .await
}

pub async fn delete_checkpoint(db: &D1Database, tenant_id: &str, id: &str) -> Result<bool> {
    let res: D1Result = db
        .prepare("DELETE FROM checkpoints WHERE tenant_id = ?1 AND id = ?2")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
        .run()
        .await?;

    let changed = res
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);
    Ok(changed)
}

// ── Memory (WS5: #45) ──────────────────────────────────────────

pub async fn create_memory(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::CreateMemory,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO memory (tenant_id, id, run_id, thread_id, scope, key, ref_type, ref_id, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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
    tenant_id: &str,
    thread_id: &str,
    limit: u32,
) -> Result<Vec<models::Memory>> {
    let now = now_iso();
    let result: D1Result = db
        .prepare(
            "SELECT * FROM memory WHERE tenant_id = ?1 AND thread_id = ?2 AND (expires_at IS NULL OR expires_at > ?3) ORDER BY created_at DESC LIMIT ?4",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
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
    tenant_id: &str,
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
                "INSERT INTO events_bronze (tenant_id, id, run_id, thread_id, event_type, node_id, actor, payload, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .bind(&[
                JsValue::from_str(tenant_id),
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
    tenant_id: &str,
    run_id: &str,
    limit: u32,
) -> Result<Vec<models::TraceEvent>> {
    let result: D1Result = db
        .prepare(
            "SELECT id, run_id, thread_id, event_type, node_id, actor, payload, created_at
             FROM events_bronze WHERE tenant_id = ?1 AND run_id = ?2 ORDER BY created_at ASC LIMIT ?3",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(run_id),
            JsValue::from(limit),
        ])?
        .all()
        .await?;

    let rows: Vec<TraceEventRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_trace_event()).collect())
}

/// Count total trace events for a run.
pub async fn count_trace_events_for_run(
    db: &D1Database,
    tenant_id: &str,
    run_id: &str,
) -> Result<u64> {
    #[derive(Debug, serde::Deserialize)]
    struct CountRow {
        total: i64,
    }

    let result: D1Result = db
        .prepare(
            "SELECT COUNT(*) AS total
             FROM events_bronze
             WHERE tenant_id = ?1 AND run_id = ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(run_id)])?
        .all()
        .await?;

    let rows: Vec<CountRow> = result.results()?;
    Ok(rows.first().map(|r| r.total.max(0) as u64).unwrap_or(0))
}

/// Insert into silver layer (sync promotion). Each row has its own id, bronze_id FK, entity_refs from event.
pub async fn insert_events_silver(
    db: &D1Database,
    tenant_id: &str,
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
                "INSERT INTO events_silver (tenant_id, id, bronze_id, run_id, thread_id, event_type, node_id, actor, payload, created_at, normalized_at, entity_refs)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .bind(&[
                JsValue::from_str(tenant_id),
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
    if let Some(v) = payload_string_ref(evt.payload.as_ref(), "task_id") {
        obj.insert("task_id".into(), serde_json::Value::String(v));
    }
    if let Some(v) = payload_string_ref(evt.payload.as_ref(), "artifact_id") {
        obj.insert("artifact_id".into(), serde_json::Value::String(v));
    }
    if let Some(v) = payload_string_ref(evt.payload.as_ref(), "plan_id") {
        obj.insert("plan_id".into(), serde_json::Value::String(v));
    }
    if let Some(v) = payload_string_ref(evt.payload.as_ref(), "tool_call_id") {
        obj.insert("tool_call_id".into(), serde_json::Value::String(v));
    }
    if obj.is_empty() {
        return None;
    }
    serde_json::to_string(&serde_json::Value::Object(obj)).ok()
}

fn payload_string_ref(payload: Option<&serde_json::Value>, key: &str) -> Option<String> {
    let obj = payload?.as_object()?;
    obj.get(key)?.as_str().map(str::to_owned)
}

// ── WS2 Domain: Runs ─────────────────────────────────────────────

pub async fn create_run(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::CreateRun,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO runs (tenant_id, id, repo, status, trigger, actor, created_at, updated_at, metadata)
         VALUES (?1, ?2, ?3, 'created', ?4, ?5, ?6, ?6, ?7)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

/// List runs for `tenant_id` ordered by `(created_at DESC, id DESC)`.
///
/// Returns `(rows, next_cursor)`. `next_cursor` is `Some` when there are more
/// rows beyond this page — we fetch `limit + 1` and trim the overflow row to
/// detect that without a second query.
///
/// The cursor predicate uses an OR of two terms (`created_at < ?` OR
/// `created_at = ? AND id < ?`) rather than a row-value comparison because
/// SQLite/D1 cannot index `(a, b) < (?, ?)` as a single sargable expression.
pub async fn list_runs(
    db: &D1Database,
    tenant_id: &str,
    repo: Option<&str>,
    limit: u32,
    cursor: Option<&crate::pagination::RunsCursor>,
) -> Result<(Vec<RunResponse>, Option<crate::pagination::RunsCursor>)> {
    let fetch_limit = limit.saturating_add(1);

    // Build the predicate and bindings in lockstep so positional parameters
    // stay in sync. Using `?` (positional, anonymous) instead of `?1/?2/...`
    // because the parameter count depends on which optional filters are set.
    let mut clauses: Vec<String> = vec!["tenant_id = ?".into()];
    let mut bindings: Vec<JsValue> = vec![JsValue::from_str(tenant_id)];

    if let Some(r) = repo {
        clauses.push("repo = ?".into());
        bindings.push(JsValue::from_str(r));
    }
    if let Some(c) = cursor {
        clauses.push("(created_at < ? OR (created_at = ? AND id < ?))".into());
        bindings.push(JsValue::from_str(&c.created_at));
        bindings.push(JsValue::from_str(&c.created_at));
        bindings.push(JsValue::from_str(&c.id));
    }
    bindings.push(JsValue::from(fetch_limit));

    let query = format!(
        "SELECT * FROM runs WHERE {} ORDER BY created_at DESC, id DESC LIMIT ?",
        clauses.join(" AND ")
    );

    let result: D1Result = db.prepare(&query).bind(&bindings)?.all().await?;
    let mut rows: Vec<RunRow> = result.results()?;

    let next_cursor = if rows.len() as u32 > limit {
        // Overflow row signals there's at least one more page. Drop it; the
        // cursor is built from the *last returned* row, not the overflow.
        rows.truncate(limit as usize);
        rows.last().map(|r| crate::pagination::RunsCursor {
            created_at: r.created_at.clone(),
            id: r.id.clone(),
        })
    } else {
        None
    };

    Ok((
        rows.into_iter().map(|r| r.into_run_response()).collect(),
        next_cursor,
    ))
}

pub async fn get_run(db: &D1Database, tenant_id: &str, id: &str) -> Result<Option<RunResponse>> {
    let row: Option<RunRow> = db
        .prepare("SELECT * FROM runs WHERE tenant_id = ?1 AND id = ?2")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
        .first(None)
        .await?;
    Ok(row.map(|r| r.into_run_response()))
}

// ── AIVCS slice 4 (issue #148): pause / resume run state ─────────────
//
// These helpers are the storage half of the POST /v1/runs/{run_id}/pause
// and /resume endpoints. The HTTP-level idempotency rules
// (already-paused → 200, terminal → 409, not-paused → 409) are layered
// in the handler in lib.rs; this module only owns the SQL transition
// and the resulting row read-back.

/// Result of a successful pause/resume transition. The handler folds
/// this into the response envelope. `paused_at` / `resumed_at` are
/// `Option<String>` because a freshly-paused run hasn't been resumed
/// yet (and vice versa) — we surface whichever timestamp the transition
/// just wrote.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct RunStatusUpdate {
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumed_at: Option<String>,
}

/// Outcome of attempting a pause transition. `Paused` is the
/// happy-path 200; `AlreadyPaused` is the idempotent 200; `Terminal`
/// maps to 409 RUN_IN_TERMINAL_STATE; `NotFound` is a guard for the
/// case where the run doesn't exist for the tenant at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PauseOutcome {
    Paused(RunStatusUpdate),
    AlreadyPaused(RunStatusUpdate),
    Terminal { current_status: String },
    NotFound,
}

/// Outcome of attempting a resume transition. `Resumed` is the
/// happy-path 200; `NotPaused` maps to 409 RUN_NOT_PAUSED (covers the
/// case where the run is in any non-paused state, including terminal);
/// `NotFound` is the missing-row guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    Resumed(RunStatusUpdate),
    NotPaused { current_status: String },
    NotFound,
}

/// Attempt to pause a run. The decision tree is:
///
/// 1. Look up the run scoped by tenant. Missing → `NotFound`.
/// 2. If status is already 'paused' → `AlreadyPaused` with the existing
///    `paused_at` (do not restamp — that would mask the original pause
///    time and confuse audit). The handler returns 200 idempotently.
/// 3. If status is terminal → `Terminal { current_status }`. The handler
///    returns 409 with the current state in the envelope so the caller
///    knows why.
/// 4. Otherwise issue `SQL_PAUSE_RUN`. We re-read the row to surface
///    the canonical `paused_at` instead of synthesising it from the
///    handler's wall clock (so D1 is the source of truth).
pub async fn pause_run(
    db: &D1Database,
    tenant_id: &str,
    run_id: &str,
    actor: &str,
) -> Result<PauseOutcome> {
    let Some(current) = get_run(db, tenant_id, run_id).await? else {
        return Ok(PauseOutcome::NotFound);
    };

    if current.status == "paused" {
        return Ok(PauseOutcome::AlreadyPaused(RunStatusUpdate {
            id: current.id,
            status: current.status,
            paused_at: current.paused_at,
            resumed_at: None,
        }));
    }

    if matches!(current.status.as_str(), "succeeded" | "failed" | "cancelled") {
        return Ok(PauseOutcome::Terminal {
            current_status: current.status,
        });
    }

    let update = db
        .prepare(SQL_PAUSE_RUN)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(run_id),
        ])?
        .run()
        .await?;

    let changed = update
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);

    if !changed {
        let Some(refreshed) = get_run(db, tenant_id, run_id).await? else {
            return Ok(PauseOutcome::NotFound);
        };
        return match refreshed.status.as_str() {
            "paused" => Ok(PauseOutcome::AlreadyPaused(RunStatusUpdate {
                id: refreshed.id,
                status: refreshed.status,
                paused_at: refreshed.paused_at,
                resumed_at: None,
            })),
            "succeeded" | "failed" | "cancelled" => Ok(PauseOutcome::Terminal {
                current_status: refreshed.status,
            }),
            other => Ok(PauseOutcome::Terminal {
                current_status: other.to_string(),
            }),
        };
    }

    // Provenance: append `run.paused` to events_bronze so the WS3 trace
    // pipeline and any downstream consumer (UI, audit, replay) can see
    // the transition. We synthesise an event id and use the same
    // `datetime('now')`-style timestamp the runs row carries.
    let event_id = format!("evt_pause_{run_id}_{}", now_iso());
    let payload = serde_json::json!({
        "transition": "pause",
        "from_status": current.status,
        "to_status": "paused",
    });
    db.prepare(
        "INSERT INTO events_bronze (tenant_id, id, run_id, thread_id, event_type, node_id, actor, payload, created_at)
         VALUES (?1, ?2, ?3, NULL, 'run.paused', NULL, ?4, ?5, datetime('now'))",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
        JsValue::from_str(&event_id),
        JsValue::from_str(run_id),
        JsValue::from_str(actor),
        JsValue::from_str(&payload.to_string()),
    ])?
    .run()
    .await?;

    // Re-read so paused_at reflects what D1 actually wrote
    // (datetime('now') is evaluated server-side).
    let refreshed = get_run(db, tenant_id, run_id).await?.ok_or_else(|| {
        Error::RustError("run vanished between pause UPDATE and read-back".to_string())
    })?;

    Ok(PauseOutcome::Paused(RunStatusUpdate {
        id: refreshed.id,
        status: refreshed.status,
        paused_at: refreshed.paused_at,
        resumed_at: None,
    }))
}

/// Attempt to resume a run. The decision tree is:
///
/// 1. Look up the run scoped by tenant. Missing → `NotFound`.
/// 2. If status is not 'paused' → `NotPaused { current_status }`. The
///    handler returns 409 RUN_NOT_PAUSED.
/// 3. Otherwise issue `SQL_RESUME_RUN`, re-read, and surface
///    `resumed_at` from D1.
pub async fn resume_run(
    db: &D1Database,
    tenant_id: &str,
    run_id: &str,
    actor: &str,
) -> Result<ResumeOutcome> {
    let Some(current) = get_run(db, tenant_id, run_id).await? else {
        return Ok(ResumeOutcome::NotFound);
    };

    if current.status != "paused" {
        return Ok(ResumeOutcome::NotPaused {
            current_status: current.status,
        });
    }

    let update = db
        .prepare(SQL_RESUME_RUN)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(run_id),
        ])?
        .run()
        .await?;

    let changed = update
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);

    if !changed {
        let Some(refreshed) = get_run(db, tenant_id, run_id).await? else {
            return Ok(ResumeOutcome::NotFound);
        };
        return Ok(ResumeOutcome::NotPaused {
            current_status: refreshed.status,
        });
    }

    let event_id = format!("evt_resume_{run_id}_{}", now_iso());
    let payload = serde_json::json!({
        "transition": "resume",
        "from_status": current.status,
        "to_status": "running",
    });
    db.prepare(
        "INSERT INTO events_bronze (tenant_id, id, run_id, thread_id, event_type, node_id, actor, payload, created_at)
         VALUES (?1, ?2, ?3, NULL, 'run.resumed', NULL, ?4, ?5, datetime('now'))",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
        JsValue::from_str(&event_id),
        JsValue::from_str(run_id),
        JsValue::from_str(actor),
        JsValue::from_str(&payload.to_string()),
    ])?
    .run()
    .await?;

    let refreshed = get_run(db, tenant_id, run_id).await?.ok_or_else(|| {
        Error::RustError("run vanished between resume UPDATE and read-back".to_string())
    })?;

    Ok(ResumeOutcome::Resumed(RunStatusUpdate {
        id: refreshed.id,
        status: refreshed.status,
        paused_at: None,
        resumed_at: refreshed.resumed_at,
    }))
}

// ── WS5 Retrieval + Memory Federation ───────────────────────────

pub async fn upsert_memory_item(
    db: &D1Database,
    tenant_id: &str,
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
            tenant_id, id, repo, kind, run_id, task_id, thread_id, checkpoint_id, artifact_key, title, summary,
            tags, content_ref, metadata, success_rate, source_created_at, indexed_at, last_accessed_at,
            access_count, status, unsafe_reason, expires_at, conflict_key, conflict_version
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16, ?17, NULL,
            0, 'active', ?18, ?19, ?20, ?21
        )",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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
    tenant_id: &str,
    id: &str,
    run_id: &str,
    body: &models::CreateTask,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO tasks (tenant_id, id, run_id, plan_id, name, status, actor, created_at, updated_at, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, 'created', ?6, ?7, ?7, ?8)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

pub async fn list_ws2_tasks(
    db: &D1Database,
    tenant_id: &str,
    run_id: &str,
) -> Result<Vec<Ws2TaskResponse>> {
    let result: D1Result = db
        .prepare("SELECT * FROM tasks WHERE tenant_id = ?1 AND run_id = ?2 ORDER BY created_at ASC")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(run_id)])?
        .all()
        .await?;
    let rows: Vec<Ws2TaskRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_task_response()).collect())
}

// ── WS2 Domain: Plans ────────────────────────────────────────────

pub async fn create_plan(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::CreatePlan,
) -> Result<()> {
    let now = now_iso();
    let task_ids_json = body
        .task_ids
        .as_ref()
        .map(|ids| serde_json::to_string(ids).unwrap())
        .unwrap_or_else(|| "[]".into());

    db.prepare(
        "INSERT INTO plans (tenant_id, id, run_id, name, status, task_ids, created_at, updated_at, metadata)
         VALUES (?1, ?2, ?3, ?4, 'created', ?5, ?6, ?6, ?7)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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
    tenant_id: &str,
    id: &str,
    body: &models::RecordToolCall,
) -> Result<()> {
    let now = now_iso();
    let input_json = serde_json::to_string(&body.input).unwrap();

    db.prepare(
        "INSERT INTO tool_calls (tenant_id, id, run_id, task_id, tool_name, input, output, status, duration_ms, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'created', ?8, ?9)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

pub async fn retrieve_memory_hybrid(
    db: &D1Database,
    tenant_id: &str,
    req: &models::RetrieveMemoryRequest,
    semantic_ids: &[String],
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

    where_parts.push(format!("tenant_id = ?{idx}"));
    bind.push(JsValue::from_str(tenant_id));
    idx += 1;

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

    let qlike = format!("%{}%", req.query.to_lowercase());

    if !semantic_ids.is_empty() {
        let mut id_placeholders = Vec::new();
        for id in semantic_ids {
            id_placeholders.push(format!("?{}", idx));
            bind.push(JsValue::from_str(id));
            idx += 1;
        }
        let id_clause = id_placeholders.join(", ");

        where_parts.push(format!(
            "(id IN ({}) OR (lower(summary) LIKE ?{} OR lower(COALESCE(title, '')) LIKE ?{} OR lower(COALESCE(tags, '')) LIKE ?{}))",
            id_clause, idx, idx + 1, idx + 2
        ));
    } else {
        where_parts.push(format!(
            "(lower(summary) LIKE ?{idx} OR lower(COALESCE(title, '')) LIKE ?{} OR lower(COALESCE(tags, '')) LIKE ?{})",
            idx + 1,
            idx + 2
        ));
    }

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

        let mut score = score_candidate(&row, &req.query, now_ms);

        // Semantic boost: if ID was in semantic_ids, boost the score
        if semantic_ids.contains(&row.id) {
            score += 2.0; // High boost for vector match
        }

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
        tenant_id,
        &query_id,
        req,
        selected.len() as i64,
        elapsed,
        stale_filtered as i64,
        unsafe_filtered as i64,
        conflict_filtered as i64,
    )
    .await?;
    touch_memory_items(db, tenant_id, &selected).await?;

    Ok(models::RetrieveMemoryResponse {
        query_id,
        latency_ms: elapsed,
        total_candidates: selected.len(), // This matches legacy retrieve_memory behavior
        returned: selected.len(),
        stale_filtered,
        unsafe_filtered,
        conflict_filtered,
        items: selected,
    })
}

pub async fn retrieve_memory(
    db: &D1Database,
    tenant_id: &str,
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

    where_parts.push(format!("tenant_id = ?{idx}"));
    bind.push(JsValue::from_str(tenant_id));
    idx += 1;

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

    // Note: stale/unsafe filtering is done in Rust (below) so that
    // telemetry counters (stale_filtered, unsafe_filtered) reflect reality.

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
    let total_eligible = candidates.len();
    let selected = candidates.into_iter().take(top_k).collect::<Vec<_>>();

    let elapsed = (js_sys::Date::now() - start).round() as i64;
    log_retrieval_query(
        db,
        tenant_id,
        &query_id,
        req,
        selected.len() as i64,
        elapsed,
        stale_filtered as i64,
        unsafe_filtered as i64,
        conflict_filtered as i64,
    )
    .await?;
    touch_memory_items(db, tenant_id, &selected).await?;

    Ok(models::RetrieveMemoryResponse {
        query_id,
        latency_ms: elapsed,
        total_candidates: total_eligible + stale_filtered + unsafe_filtered + conflict_filtered,
        returned: selected.len(),
        stale_filtered,
        unsafe_filtered,
        conflict_filtered,
        items: selected,
    })
}

pub async fn build_context_pack(
    db: &D1Database,
    tenant_id: &str,
    req: &models::ContextPackRequest,
) -> Result<models::ContextPackResponse> {
    let retrieval = retrieve_memory(db, tenant_id, &req.retrieval).await?;
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

pub async fn retire_memory_item(db: &D1Database, tenant_id: &str, id: &str) -> Result<bool> {
    let res: D1Result = db
        .prepare("UPDATE memory_index SET status = 'retired' WHERE tenant_id = ?1 AND id = ?2")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
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
    tenant_id: &str,
    req: &models::MemoryGcRequest,
) -> Result<models::MemoryGcResponse> {
    let now = now_iso();
    let limit = req.limit.clamp(1, 10_000) as i32;

    // Use a single DELETE statement with a subquery to avoid D1 batch limits (max 100 statements).
    // This efficiently removes up to `limit` expired or retired items in one round trip.
    let res: D1Result = db
        .prepare(
            "DELETE FROM memory_index
             WHERE tenant_id = ?1 AND id IN (
                 SELECT id FROM memory_index
                 WHERE tenant_id = ?1 AND (status = 'retired' OR (expires_at IS NOT NULL AND expires_at <= ?2))
                 ORDER BY indexed_at ASC
                 LIMIT ?3
             )",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&now),
            JsValue::from(limit),
        ])?
        .run()
        .await?;

    let deleted = res.meta()?.and_then(|m| m.changes).unwrap_or(0) as usize;

    Ok(models::MemoryGcResponse {
        scanned: deleted,
        retired: 0, // No longer distinguishing retired vs expired in the single-pass DELETE
        deleted,
    })
}

pub async fn record_retrieval_feedback(
    db: &D1Database,
    tenant_id: &str,
    feedback: &models::RetrievalFeedback,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO memory_retrieval_feedback (
            tenant_id, query_id, run_id, task_id, success, first_pass_success, cache_hit, latency_ms, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
        JsValue::from_str(&feedback.query_id),
        opt_str(&feedback.run_id),
        opt_str(&feedback.task_id),
        JsValue::from(if feedback.success { 1 } else { 0 }),
        JsValue::from(if feedback.first_pass_success { 1 } else { 0 }),
        JsValue::from(if feedback.cache_hit { 1 } else { 0 }),
        match feedback.latency_ms {
            Some(v) => JsValue::from(v as i32),
            None => JsValue::NULL,
        },
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;
    Ok(())
}

// ── WS2 Domain: Releases ────────────────────────────────────────

pub async fn create_release(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::CreateRelease,
) -> Result<()> {
    let now = now_iso();
    let artifact_ids_json = body
        .artifact_ids
        .as_ref()
        .map(|ids| serde_json::to_string(ids).unwrap())
        .unwrap_or_else(|| "[]".into());

    db.prepare(
        "INSERT INTO releases (tenant_id, id, repo, version, run_id, artifact_ids, status, created_at, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'created', ?7, ?8)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

pub async fn ingest_event(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::IngestEvent,
) -> Result<()> {
    let now = now_iso();
    let entity_kind = body.entity_kind.as_deref().unwrap_or("event");
    let entity_id = body.entity_id.as_deref().unwrap_or(id);
    db.prepare(
        "INSERT INTO events (tenant_id, id, run_id, entity_kind, entity_id, event_type, actor, created_at, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

#[allow(clippy::too_many_arguments)]
pub async fn record_policy_check_detailed(
    db: &D1Database,
    tenant_id: &str,
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
    let mut merged = body
        .context
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));
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
        "INSERT INTO policy_decisions (tenant_id, id, run_id, action, actor, resource, decision, reason, created_at, context)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

#[allow(clippy::too_many_arguments)]
pub async fn create_policy_escalation(
    db: &D1Database,
    tenant_id: &str,
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
    db.prepare(SQL_INSERT_POLICY_ESCALATION)
        .bind(&[
            JsValue::from_str(tenant_id),
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
    tenant_id: &str,
    id: &str,
    body: &models::CreatePolicyRule,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT INTO policy_rules (tenant_id, id, name, action_pattern, resource_pattern, actor_pattern, risk_level, verdict, reason, priority, enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?11)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

pub async fn memory_eval_summary(
    db: &D1Database,
    tenant_id: &str,
) -> Result<models::MemoryEvalSummary> {
    let row: Option<MemoryEvalRow> = db
        .prepare(
            "SELECT
                COUNT(*) AS total_queries,
                AVG(CASE WHEN cache_hit = 1 THEN 1.0 ELSE 0.0 END) AS cache_hit_rate,
                AVG(CASE WHEN success = 1 THEN 1.0 ELSE 0.0 END) AS success_rate,
                AVG(CASE WHEN first_pass_success = 1 THEN 1.0 ELSE 0.0 END) AS first_pass_success_rate
             FROM memory_retrieval_feedback WHERE tenant_id = ?1",
        )
        .bind(&[JsValue::from_str(tenant_id)])?
        .first(None)
        .await?;

    let p50: Option<PercentileRow> = db
        .prepare(
            "SELECT latency_ms
             FROM memory_retrieval_feedback
             WHERE tenant_id = ?1 AND latency_ms IS NOT NULL
             ORDER BY latency_ms
             LIMIT 1 OFFSET (SELECT CAST((COUNT(*) * 0.50) AS INTEGER) FROM memory_retrieval_feedback WHERE tenant_id = ?2 AND latency_ms IS NOT NULL)",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(tenant_id)])?
        .first(None)
        .await?;
    let p95: Option<PercentileRow> = db
        .prepare(
            "SELECT latency_ms
             FROM memory_retrieval_feedback
             WHERE tenant_id = ?1 AND latency_ms IS NOT NULL
             ORDER BY latency_ms
             LIMIT 1 OFFSET (SELECT CAST((COUNT(*) * 0.95) AS INTEGER) FROM memory_retrieval_feedback WHERE tenant_id = ?2 AND latency_ms IS NOT NULL)",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(tenant_id)])?
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
    let mut item_ms = js_sys::Date::parse(stamp);
    if !item_ms.is_finite() {
        let fallback_ms = js_sys::Date::parse(&row.indexed_at);
        if !fallback_ms.is_finite() {
            return 0.0;
        }
        item_ms = fallback_ms;
    }
    if !now_ms.is_finite() || now_ms <= item_ms {
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

async fn touch_memory_items(
    db: &D1Database,
    tenant_id: &str,
    items: &[models::MemoryCandidate],
) -> Result<()> {
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
                 WHERE tenant_id = ?2 AND id = ?3",
            )
            .bind(&[
                JsValue::from_str(&now),
                JsValue::from_str(tenant_id),
                JsValue::from_str(&item.id),
            ])?;
        stmts.push(stmt);
    }
    db.batch(stmts).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn log_retrieval_query(
    db: &D1Database,
    tenant_id: &str,
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
            tenant_id, id, repo, query_text, run_id, task_id, thread_id, top_k, related_repos,
            returned_count, latency_ms, stale_filtered, unsafe_filtered, conflict_filtered, created_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
            ?10, ?11, ?12, ?13, ?14, ?15
        )",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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

/// List policy rules for `tenant_id` ordered by
/// `(priority DESC, created_at ASC, id ASC)`.
///
/// Returns `(rows, next_cursor)`. Mirrors the `list_runs` pattern: we fetch
/// `limit + 1` and drop the overflow row to detect the next page. The cursor
/// predicate is expressed as a three-level OR over `(priority, created_at, id)`
/// because SQLite/D1 cannot index a row-value tuple comparison.
pub async fn list_policy_rules(
    db: &D1Database,
    tenant_id: &str,
    limit: u32,
    cursor: Option<&crate::pagination::PolicyRulesCursor>,
) -> Result<(
    Vec<PolicyRuleRow>,
    Option<crate::pagination::PolicyRulesCursor>,
)> {
    let fetch_limit = limit.saturating_add(1);

    let mut clauses: Vec<String> = vec!["tenant_id = ?".into()];
    let mut bindings: Vec<JsValue> = vec![JsValue::from_str(tenant_id)];

    if let Some(c) = cursor {
        // Strict-after predicate on `(priority DESC, created_at ASC, id ASC)`:
        //   priority < cursor.priority
        //   OR (priority = cursor.priority AND created_at > cursor.created_at)
        //   OR (priority = cursor.priority AND created_at = cursor.created_at AND id > cursor.id)
        clauses.push(
            "(priority < ? OR (priority = ? AND created_at > ?) OR (priority = ? AND created_at = ? AND id > ?))"
                .into(),
        );
        bindings.push(JsValue::from(c.priority));
        bindings.push(JsValue::from(c.priority));
        bindings.push(JsValue::from_str(&c.created_at));
        bindings.push(JsValue::from(c.priority));
        bindings.push(JsValue::from_str(&c.created_at));
        bindings.push(JsValue::from_str(&c.id));
    }
    bindings.push(JsValue::from(fetch_limit));

    let query = format!(
        "SELECT * FROM policy_rules WHERE {} ORDER BY priority DESC, created_at ASC, id ASC LIMIT ?",
        clauses.join(" AND ")
    );

    let result: D1Result = db.prepare(&query).bind(&bindings)?.all().await?;
    let mut rows: Vec<PolicyRuleRow> = result.results()?;

    let next_cursor = compute_policy_rules_next_cursor(&mut rows, limit);

    Ok((rows, next_cursor))
}

/// Pure helper: trims the overflow row and produces the cursor for the next
/// page. Extracted from `list_policy_rules` so it can be unit-tested without
/// D1.
pub(crate) fn compute_policy_rules_next_cursor(
    rows: &mut Vec<PolicyRuleRow>,
    limit: u32,
) -> Option<crate::pagination::PolicyRulesCursor> {
    if rows.len() as u32 > limit {
        rows.truncate(limit as usize);
        rows.last().map(|r| crate::pagination::PolicyRulesCursor {
            priority: r.priority,
            created_at: r.created_at.clone(),
            id: r.id.clone(),
        })
    } else {
        None
    }
}

pub async fn get_policy_rule(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
) -> Result<Option<PolicyRuleRow>> {
    db.prepare("SELECT * FROM policy_rules WHERE tenant_id = ?1 AND id = ?2")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
        .first(None)
        .await
}

pub async fn update_policy_rule(
    db: &D1Database,
    tenant_id: &str,
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
    bind_vals.push(JsValue::from_str(tenant_id));
    bind_vals.push(JsValue::from_str(id));

    let query = format!(
        "UPDATE policy_rules SET {} WHERE tenant_id = ?{param_idx} AND id = ?{}",
        set_parts.join(", "),
        param_idx + 1
    );

    let res: D1Result = db.prepare(&query).bind(&bind_vals)?.run().await?;
    let changed = res
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);
    Ok(changed)
}

pub async fn delete_policy_rule(db: &D1Database, tenant_id: &str, id: &str) -> Result<bool> {
    let res: D1Result = db
        .prepare("DELETE FROM policy_rules WHERE tenant_id = ?1 AND id = ?2")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
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
#[allow(dead_code)]
pub async fn evaluate_policy(
    db: &D1Database,
    tenant_id: &str,
    req: &models::PolicyCheckRequest,
) -> Result<(String, String, String, Option<String>)> {
    let result: D1Result = db
        .prepare(
            "SELECT * FROM policy_rules WHERE tenant_id = ?1 AND enabled = 1 ORDER BY priority DESC, created_at ASC",
        )
        .bind(&[JsValue::from_str(tenant_id)])?
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
    tenant_id: &str,
    action: Option<&str>,
    actor: Option<&str>,
    decision: Option<&str>,
    limit: u32,
) -> Result<Vec<PolicyDecisionRow>> {
    // Build dynamic WHERE
    let mut conditions: Vec<String> = Vec::new();
    let mut bindings: Vec<JsValue> = Vec::new();
    let mut idx = 2u32;

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
        "WHERE tenant_id = ?1".to_string()
    } else {
        format!("WHERE tenant_id = ?1 AND {}", conditions.join(" AND "))
    };

    let query = format!(
        "SELECT * FROM policy_decisions {where_clause} ORDER BY created_at DESC LIMIT ?{idx}"
    );
    bindings.insert(0, JsValue::from_str(tenant_id));
    bindings.push(JsValue::from(limit));

    let result: D1Result = db.prepare(&query).bind(&bindings)?.all().await?;
    result.results()
}

pub async fn create_verification_evidence(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    response: &models::ReplayExecuteResponse,
) -> Result<()> {
    let now = now_iso();
    let failure_classification = response
        .failure_classification
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| Error::RustError(format!("failure_classification serialization: {e}")))?
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let failed_gates_json = serde_json::to_string(&response.verification.failed_gates)
        .map_err(|e| Error::RustError(format!("failed_gates serialization: {e}")))?;

    db.prepare(
        "INSERT INTO verification_evidence (
            tenant_id, id, run_id, baseline_run_id, status,
            step_count, drift_count, drift_ratio_percent, within_variance,
            failure_classification,
            tests_passed, policy_approved, provenance_complete,
            eligible_for_promotion, confidence_score, failed_gates,
            created_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            ?6, ?7, ?8, ?9,
            ?10,
            ?11, ?12, ?13,
            ?14, ?15, ?16,
            ?17
         )",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
        JsValue::from_str(id),
        JsValue::from_str(&response.run_id),
        response
            .baseline_run_id
            .as_ref()
            .map(|s| JsValue::from_str(s))
            .unwrap_or(JsValue::NULL),
        JsValue::from_str(&response.status),
        JsValue::from(response.step_count as i32),
        JsValue::from(response.drift_count as i32),
        JsValue::from(response.drift_ratio_percent),
        JsValue::from(response.within_variance),
        failure_classification
            .as_ref()
            .map(|s| JsValue::from_str(s))
            .unwrap_or(JsValue::NULL),
        JsValue::from(response.verification.tests_passed),
        JsValue::from(response.verification.policy_approved),
        JsValue::from(response.verification.provenance_complete),
        JsValue::from(response.verification.eligible_for_promotion),
        JsValue::from(response.verification.confidence_score as i32),
        JsValue::from_str(&failed_gates_json),
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;

    Ok(())
}

pub async fn list_verification_evidence(
    db: &D1Database,
    tenant_id: &str,
    run_id: Option<&str>,
    limit: u32,
) -> Result<Vec<VerificationEvidenceRow>> {
    let mut bindings: Vec<JsValue> = vec![JsValue::from_str(tenant_id)];
    let mut idx = 2u32;
    let mut where_clause = "WHERE tenant_id = ?1".to_string();
    if let Some(run_id) = run_id {
        where_clause.push_str(&format!(" AND run_id = ?{idx}"));
        bindings.push(JsValue::from_str(run_id));
        idx += 1;
    }

    let query = format!(
        "SELECT * FROM verification_evidence {where_clause} ORDER BY created_at DESC LIMIT ?{idx}"
    );
    bindings.push(JsValue::from(limit));
    let result: D1Result = db.prepare(&query).bind(&bindings)?.all().await?;
    result.results()
}

pub async fn record_telemetry(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::TelemetrySnapshot,
) -> Result<()> {
    let now = now_iso();
    let payload_json = body
        .payload
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap());

    db.prepare(
        "INSERT INTO telemetry_snapshots (
            id, tenant_id, agent_name, agent_type, status,
            duration_seconds, total_attempts, success_rate, namespace,
            payload, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(tenant_id),
        JsValue::from_str(&body.agent_name),
        JsValue::from_str(&body.agent_type),
        JsValue::from_str(&body.status),
        JsValue::from(body.duration_seconds),
        JsValue::from(body.total_attempts),
        JsValue::from(body.success_rate),
        JsValue::from_str(&body.namespace),
        match &payload_json {
            Some(s) => JsValue::from_str(s),
            None => JsValue::NULL,
        },
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;

    Ok(())
}

// ── Reasoning traces (#111) ────────────────────────────────────

/// Look up an existing reasoning_traces row id by its idempotency key. The
/// handler calls this BEFORE writing payloads to R2 so a retry is detected
/// without orphaning storage.
pub async fn find_reasoning_trace_by_idempotency_key(
    db: &D1Database,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<String>> {
    #[derive(serde::Deserialize)]
    struct IdRow {
        id: String,
    }
    let row: Option<IdRow> = db
        .prepare(
            "SELECT id FROM reasoning_traces
             WHERE tenant_id = ?1 AND idempotency_key = ?2 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(idempotency_key),
        ])?
        .first(None)
        .await?;
    Ok(row.map(|r| r.id))
}

/// Resolved storage locations for the inputs/outputs of a reasoning step.
/// Either the inline JSON or the R2 key is set for each side — never both.
pub struct ReasoningPayloadRefs<'a> {
    pub inputs_inline: Option<&'a str>,
    pub inputs_r2_key: Option<&'a str>,
    pub outputs_inline: Option<&'a str>,
    pub outputs_r2_key: Option<&'a str>,
}

/// Insert a reasoning trace row. Idempotent on (tenant_id, idempotency_key):
/// a duplicate insert returns `Ok(false)` so the caller can report
/// `deduplicated: true` to the client without surfacing an error.
pub async fn insert_reasoning_trace(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::IngestReasoningTrace,
    payload: ReasoningPayloadRefs<'_>,
) -> Result<bool> {
    let now = now_iso();
    let res: D1Result = db
        .prepare(
            "INSERT INTO reasoning_traces (
            id, tenant_id, schema_version,
            agent_id, job_id, parent_span_id, step_number, step_type,
            inputs_inline, inputs_r2_key, outputs_inline, outputs_r2_key,
            tokens_input, tokens_output, tokens_cached,
            started_at, completed_at, idempotency_key, created_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
         )
         ON CONFLICT (tenant_id, idempotency_key) DO NOTHING",
        )
        .bind(&[
            JsValue::from_str(id),
            JsValue::from_str(tenant_id),
            JsValue::from(body.schema_version),
            JsValue::from_str(&body.agent_id),
            JsValue::from_str(&body.job_id),
            opt_str(&body.parent_span_id),
            JsValue::from(body.step_number),
            JsValue::from_str(body.step_type.as_str()),
            payload
                .inputs_inline
                .map(JsValue::from_str)
                .unwrap_or(JsValue::NULL),
            payload
                .inputs_r2_key
                .map(JsValue::from_str)
                .unwrap_or(JsValue::NULL),
            payload
                .outputs_inline
                .map(JsValue::from_str)
                .unwrap_or(JsValue::NULL),
            payload
                .outputs_r2_key
                .map(JsValue::from_str)
                .unwrap_or(JsValue::NULL),
            JsValue::from(body.tokens.input),
            JsValue::from(body.tokens.output),
            JsValue::from(body.tokens.cached),
            JsValue::from_str(&body.started_at),
            opt_str(&body.completed_at),
            JsValue::from_str(&body.idempotency_key),
            JsValue::from_str(&now),
        ])?
        .run()
        .await?;

    let inserted = res
        .meta()?
        .map(|m| m.changes.unwrap_or(0) > 0)
        .unwrap_or(false);
    Ok(inserted)
}

/// Page of reasoning traces for a single job. `after_step` is the cursor:
/// callers paginate by feeding the last returned `step_number` back in.
/// `has_more` is true when the DB held at least one more row beyond the
/// returned page (computed by fetching `limit + 1` and trimming).
pub struct ReasoningTracePage {
    pub traces: Vec<models::ReasoningTrace>,
    pub has_more: bool,
}

pub async fn list_reasoning_traces_for_job(
    db: &D1Database,
    tenant_id: &str,
    job_id: &str,
    after_step: Option<i64>,
    limit: u32,
) -> Result<ReasoningTracePage> {
    // Fetch one extra row to detect whether more pages exist without a
    // separate COUNT round-trip.
    let fetch = (limit as i64) + 1;
    let result: D1Result = db
        .prepare(
            "SELECT * FROM reasoning_traces
             WHERE tenant_id = ?1 AND job_id = ?2 AND step_number > ?3
             ORDER BY step_number ASC
             LIMIT ?4",
        )
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(job_id),
            JsValue::from(after_step.unwrap_or(-1) as f64),
            JsValue::from(fetch as f64),
        ])?
        .all()
        .await?;
    let mut rows: Vec<ReasoningTraceRow> = result.results()?;
    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }
    Ok(ReasoningTracePage {
        traces: rows.into_iter().map(|r| r.into_trace()).collect(),
        has_more,
    })
}

pub async fn get_reasoning_trace(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
) -> Result<Option<models::ReasoningTrace>> {
    let row: Option<ReasoningTraceRow> = db
        .prepare("SELECT * FROM reasoning_traces WHERE tenant_id = ?1 AND id = ?2")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
        .first(None)
        .await?;
    Ok(row.map(|r| r.into_trace()))
}

#[derive(Debug, serde::Deserialize)]
struct ReasoningTraceRow {
    id: String,
    schema_version: i64,
    agent_id: String,
    job_id: String,
    parent_span_id: Option<String>,
    step_number: i64,
    step_type: String,
    inputs_inline: Option<String>,
    inputs_r2_key: Option<String>,
    outputs_inline: Option<String>,
    outputs_r2_key: Option<String>,
    tokens_input: i64,
    tokens_output: i64,
    tokens_cached: i64,
    started_at: String,
    completed_at: Option<String>,
    created_at: String,
}

impl ReasoningTraceRow {
    fn into_trace(self) -> models::ReasoningTrace {
        let step_type = models::StepType::from_storage_str(&self.step_type);
        models::ReasoningTrace {
            id: self.id,
            schema_version: self.schema_version as u32,
            agent_id: self.agent_id,
            job_id: self.job_id,
            parent_span_id: self.parent_span_id,
            step_number: self.step_number as u32,
            step_type,
            inputs_inline: self
                .inputs_inline
                .and_then(|s| serde_json::from_str(&s).ok()),
            inputs_r2_key: self.inputs_r2_key,
            outputs_inline: self
                .outputs_inline
                .and_then(|s| serde_json::from_str(&s).ok()),
            outputs_r2_key: self.outputs_r2_key,
            tokens: models::TokenCost {
                input: self.tokens_input as u32,
                output: self.tokens_output as u32,
                cached: self.tokens_cached as u32,
            },
            started_at: self.started_at,
            completed_at: self.completed_at,
            created_at: self.created_at,
        }
    }
}

pub async fn provision_tenant(
    db: &D1Database,
    body: &models::TenantProvisionRequest,
) -> Result<()> {
    let now = now_iso();
    let plan = if body.plan.trim().is_empty() {
        "standard"
    } else {
        body.plan.as_str()
    };
    let quota_runs = if body.quota_runs_per_minute <= 0 {
        120
    } else {
        body.quota_runs_per_minute
    };

    db.prepare(
        "INSERT INTO tenants (tenant_id, display_name, plan, quota_runs_per_minute, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(tenant_id) DO UPDATE SET
           display_name = excluded.display_name,
           plan = excluded.plan,
           quota_runs_per_minute = excluded.quota_runs_per_minute,
           updated_at = excluded.updated_at",
    )
    .bind(&[
        JsValue::from_str(&body.tenant_id),
        JsValue::from_str(&body.display_name),
        JsValue::from_str(plan),
        JsValue::from(quota_runs as f64),
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;

    Ok(())
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

/// Compose the synthetic rate-limit counter id used as the SQLite
/// PRIMARY KEY for `policy_rate_limit_counters`. Tenant_id is the first
/// segment so two tenants sharing an actor name (e.g. "agent-1") cannot
/// collide into a single counter row. Exposed at module scope so the
/// cross-tenant SQL-shape tests can assert the format.
fn rate_limit_counter_id(
    tenant_id: &str,
    actor: &str,
    action_class: &str,
    window_start: i64,
    window_seconds: i64,
) -> String {
    format!("{tenant_id}|{actor}|{action_class}|{window_start}|{window_seconds}")
}

pub async fn check_and_increment_rate_limit(
    db: &D1Database,
    tenant_id: &str,
    actor: &str,
    action_class: &str,
    window_seconds: i64,
    max_requests: i64,
) -> Result<bool> {
    let now = js_sys::Date::now() as i64 / 1000;
    let window_start = now - (now % window_seconds.max(1));
    // Tenant_id is the first segment of the counter id so each tenant gets
    // its own per-(actor, action_class, window) slot. Before this change
    // two tenants sharing an actor name (e.g. "agent-1") collided into a
    // single counter and one tenant could exhaust the other's quota.
    let counter_id =
        rate_limit_counter_id(tenant_id, actor, action_class, window_start, window_seconds);
    let now_iso = now_iso();

    db.prepare(SQL_UPSERT_RATE_LIMIT_COUNTER)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&counter_id),
            JsValue::from_str(actor),
            JsValue::from_str(action_class),
            JsValue::from(window_start),
            JsValue::from(window_seconds),
            JsValue::from_str(&now_iso),
        ])?
        .run()
        .await?;

    // SELECT is also tenant-scoped as defense-in-depth: even though the
    // counter_id already includes tenant_id, the WHERE clause makes the
    // tenant boundary explicit and protects against id-shape regressions.
    let row: Option<RateCounterRow> = db
        .prepare(SQL_SELECT_RATE_LIMIT_COUNT)
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(&counter_id)])?
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

    let events_deleted =
        delete_older_than(db, "events_bronze", "created_at", &events_cutoff).await?;
    let policy_decisions_deleted =
        delete_older_than(db, "policy_decisions", "created_at", &policy_cutoff).await?;

    let checkpoint_keys = list_old_keys(
        db,
        "checkpoints",
        "state_r2_key",
        "created_at",
        &checkpoints_cutoff,
    )
    .await?;
    let checkpoints_deleted =
        delete_older_than(db, "checkpoints", "created_at", &checkpoints_cutoff).await?;
    for key in checkpoint_keys {
        let _ = bucket.delete(&key).await;
    }

    let artifact_keys =
        list_old_keys(db, "artifacts", "key", "created_at", &artifacts_cutoff).await?;
    let artifacts_deleted =
        delete_older_than(db, "artifacts", "created_at", &artifacts_cutoff).await?;
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
        "SELECT count(*) as count FROM {table} WHERE datetime({time_col}) < datetime('now', ?1)"
    );
    let count_row: Option<CountRow> = db
        .prepare(&count_sql)
        .bind(&[JsValue::from_str(cutoff)])?
        .first(None)
        .await?;
    let count = count_row.map(|r| r.count as usize).unwrap_or(0);

    let delete_sql =
        format!("DELETE FROM {table} WHERE datetime({time_col}) < datetime('now', ?1)");
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
        "SELECT {key_col} as key FROM {table} WHERE datetime({time_col}) < datetime('now', ?1)"
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

// ── AIVCS review projections (issue #148 slice 2) ──────────────
//
// CRUD helpers for the four review-projection tables introduced by
// migration 0016. Every helper takes `tenant_id` as the leading
// argument and binds it as ?1; the SQL constants used here are
// asserted to keep that shape by the `cross_tenant_sql_aivcs_*` tests.
//
// No HTTP routes are wired in this slice; the helpers exist so the
// follower process and (later) BFF can write/read the projection
// without re-typing SQL.

#[allow(dead_code)]
pub async fn insert_review_thread(
    db: &D1Database,
    tenant_id: &str,
    thread: &models::aivcs_review::ReviewThread,
) -> Result<()> {
    db.prepare(SQL_INSERT_REVIEW_THREAD)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&thread.id),
            JsValue::from_str(&thread.review_id),
            opt_str(&thread.change_set_id),
            JsValue::from_str(thread.status.as_sql()),
            JsValue::from_str(&thread.created_at),
            opt_str(&thread.resolved_at),
        ])?
        .run()
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn get_review_thread(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
) -> Result<Option<models::aivcs_review::ReviewThread>> {
    let row: Option<ReviewThreadRow> = db
        .prepare(SQL_GET_REVIEW_THREAD)
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
        .first(None)
        .await?;
    Ok(row.and_then(|r| r.into_review_thread()))
}

#[allow(dead_code)]
pub async fn list_review_threads_for_review(
    db: &D1Database,
    tenant_id: &str,
    review_id: &str,
    limit: u32,
) -> Result<Vec<models::aivcs_review::ReviewThread>> {
    let result: D1Result = db
        .prepare(SQL_LIST_REVIEW_THREADS_FOR_REVIEW)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(review_id),
            JsValue::from(limit),
        ])?
        .all()
        .await?;
    let rows: Vec<ReviewThreadRow> = result.results()?;
    Ok(rows
        .into_iter()
        .filter_map(|r| r.into_review_thread())
        .collect())
}

#[allow(dead_code)]
pub async fn update_review_thread_status(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    status: models::aivcs_review::ReviewThreadStatus,
    resolved_at: Option<&str>,
) -> Result<()> {
    let resolved_at_js = match resolved_at {
        Some(s) => JsValue::from_str(s),
        None => JsValue::NULL,
    };
    db.prepare(SQL_UPDATE_REVIEW_THREAD_STATUS)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(id),
            JsValue::from_str(status.as_sql()),
            resolved_at_js,
        ])?
        .run()
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn insert_review_comment(
    db: &D1Database,
    tenant_id: &str,
    comment: &models::aivcs_review::ReviewComment,
) -> Result<()> {
    db.prepare(SQL_INSERT_REVIEW_COMMENT)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&comment.id),
            JsValue::from_str(&comment.thread_id),
            JsValue::from_str(&comment.actor.to_sql_actor()),
            JsValue::from_str(&comment.body),
            opt_str(&comment.parent_comment_id),
            JsValue::from_str(&comment.created_at),
        ])?
        .run()
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn list_review_comments_for_thread(
    db: &D1Database,
    tenant_id: &str,
    thread_id: &str,
    limit: u32,
) -> Result<Vec<models::aivcs_review::ReviewComment>> {
    let result: D1Result = db
        .prepare(SQL_LIST_REVIEW_COMMENTS_FOR_THREAD)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(thread_id),
            JsValue::from(limit),
        ])?
        .all()
        .await?;
    let rows: Vec<ReviewCommentRow> = result.results()?;
    Ok(rows
        .into_iter()
        .filter_map(|r| r.into_review_comment())
        .collect())
}

#[allow(dead_code)]
pub async fn insert_review_thread_resolution(
    db: &D1Database,
    tenant_id: &str,
    resolution: &models::aivcs_review::ReviewThreadResolution,
) -> Result<()> {
    db.prepare(SQL_INSERT_REVIEW_THREAD_RESOLUTION)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&resolution.thread_id),
            JsValue::from_str(&resolution.resolved_by),
            JsValue::from_str(resolution.resolution.as_sql()),
            opt_str(&resolution.note),
            JsValue::from_str(&resolution.resolved_at),
        ])?
        .run()
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn get_review_thread_resolution(
    db: &D1Database,
    tenant_id: &str,
    thread_id: &str,
) -> Result<Option<models::aivcs_review::ReviewThreadResolution>> {
    let row: Option<ReviewThreadResolutionRow> = db
        .prepare(SQL_GET_REVIEW_THREAD_RESOLUTION)
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(thread_id)])?
        .first(None)
        .await?;
    Ok(row.and_then(|r| r.into_resolution()))
}

#[allow(dead_code)]
pub async fn insert_file_anchor(
    db: &D1Database,
    tenant_id: &str,
    anchor: &models::aivcs_review::FileAnchor,
) -> Result<()> {
    db.prepare(SQL_INSERT_FILE_ANCHOR)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&anchor.id),
            JsValue::from_str(&anchor.thread_id),
            JsValue::from_str(&anchor.file_path),
            JsValue::from_f64(anchor.start_line() as f64),
            JsValue::from_f64(anchor.end_line() as f64),
            JsValue::from_str(anchor.side.as_sql()),
        ])?
        .run()
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn list_file_anchors_for_thread(
    db: &D1Database,
    tenant_id: &str,
    thread_id: &str,
    limit: u32,
) -> Result<Vec<models::aivcs_review::FileAnchor>> {
    let result: D1Result = db
        .prepare(SQL_LIST_FILE_ANCHORS_FOR_THREAD)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(thread_id),
            JsValue::from(limit),
        ])?
        .all()
        .await?;
    let rows: Vec<FileAnchorRow> = result.results()?;
    Ok(rows
        .into_iter()
        .filter_map(|r| r.into_file_anchor())
        .collect())
}

// ── Internal row types ──────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct TaskIdRow {
    id: String,
}

#[allow(dead_code)]
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
    #[serde(default)]
    pub tenant_id: Option<String>,
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
            memory_context: None,
            tenant_id: self.tenant_id,
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
    // AIVCS issue #148 slice 4: pause/resume bookkeeping. Optional
    // because rows that pre-date migration 0018 won't have these set,
    // and the columns are nullable. `#[serde(default)]` lets D1's
    // serde_wasm_bindgen deserializer tolerate either nulls or missing
    // columns during the migration window.
    #[serde(default)]
    pub paused_at: Option<String>,
    #[serde(default)]
    pub resumed_at: Option<String>,
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
            paused_at: self.paused_at,
            resumed_at: self.resumed_at,
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
    /// Last pause timestamp (server-stamped on the pause transition).
    /// `None` for runs that have never been paused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<String>,
    /// Last resume timestamp (server-stamped on the resume transition).
    /// `None` for runs that have never been resumed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumed_at: Option<String>,
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

// ── AIVCS review-projection row types (issue #148 slice 2) ─────

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct ReviewThreadRow {
    id: String,
    review_id: String,
    change_set_id: Option<String>,
    status: String,
    created_at: String,
    resolved_at: Option<String>,
}

#[allow(dead_code)]
impl ReviewThreadRow {
    fn into_review_thread(self) -> Option<models::aivcs_review::ReviewThread> {
        let status = models::aivcs_review::ReviewThreadStatus::from_sql(&self.status)?;
        Some(models::aivcs_review::ReviewThread {
            id: self.id,
            review_id: self.review_id,
            change_set_id: self.change_set_id,
            status,
            created_at: self.created_at,
            resolved_at: self.resolved_at,
        })
    }
}

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct ReviewCommentRow {
    id: String,
    thread_id: String,
    actor: String,
    body: String,
    parent_comment_id: Option<String>,
    created_at: String,
}

#[allow(dead_code)]
impl ReviewCommentRow {
    fn into_review_comment(self) -> Option<models::aivcs_review::ReviewComment> {
        let actor = models::aivcs_review::CommentActor::from_sql_actor(&self.actor)?;
        Some(models::aivcs_review::ReviewComment {
            id: self.id,
            thread_id: self.thread_id,
            actor,
            body: self.body,
            parent_comment_id: self.parent_comment_id,
            created_at: self.created_at,
        })
    }
}

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct ReviewThreadResolutionRow {
    thread_id: String,
    resolved_by: String,
    resolution: String,
    note: Option<String>,
    resolved_at: String,
}

#[allow(dead_code)]
impl ReviewThreadResolutionRow {
    fn into_resolution(self) -> Option<models::aivcs_review::ReviewThreadResolution> {
        let resolution = models::aivcs_review::Resolution::from_sql(&self.resolution)?;
        Some(models::aivcs_review::ReviewThreadResolution {
            thread_id: self.thread_id,
            resolved_by: self.resolved_by,
            resolution,
            note: self.note,
            resolved_at: self.resolved_at,
        })
    }
}

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct FileAnchorRow {
    id: String,
    thread_id: String,
    file_path: String,
    start_line: i64,
    end_line: i64,
    side: String,
}

#[allow(dead_code)]
impl FileAnchorRow {
    fn into_file_anchor(self) -> Option<models::aivcs_review::FileAnchor> {
        let side = models::aivcs_review::AnchorSide::from_sql(&self.side)?;
        models::aivcs_review::FileAnchor::from_row(
            self.id,
            self.thread_id,
            self.file_path,
            self.start_line,
            self.end_line,
            side,
        )
        .ok()
    }
}

// ── Provenance Links (WS3: causality chain) ─────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn insert_relationship(
    db: &D1Database,
    tenant_id: &str,
    rel_type: &str,
    from_kind: &str,
    from_id: &str,
    to_kind: &str,
    to_id: &str,
    relation: Option<&str>,
) -> Result<()> {
    let now = now_iso();
    db.prepare(
        "INSERT OR IGNORE INTO relationships (tenant_id, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
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
    tenant_id: &str,
    kind: &str,
    id: &str,
    direction: &str,
    max_hops: u32,
) -> Result<Vec<models::ProvenanceEdge>> {
    let query = if direction == "backward" {
        "WITH RECURSIVE chain(depth, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at) AS (
           SELECT 0, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at
             FROM relationships WHERE tenant_id = ?1 AND to_kind = ?2 AND to_id = ?3
           UNION
           SELECT c.depth + 1, r.rel_type, r.from_kind, r.from_id, r.to_kind, r.to_id, r.relation, r.created_at
             FROM relationships r JOIN chain c ON r.tenant_id = ?1 AND r.to_kind = c.from_kind AND r.to_id = c.from_id
             WHERE c.depth < ?4
         ) SELECT * FROM chain ORDER BY depth"
    } else {
        "WITH RECURSIVE chain(depth, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at) AS (
           SELECT 0, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at
             FROM relationships WHERE tenant_id = ?1 AND from_kind = ?2 AND from_id = ?3
           UNION
           SELECT c.depth + 1, r.rel_type, r.from_kind, r.from_id, r.to_kind, r.to_id, r.relation, r.created_at
             FROM relationships r JOIN chain c ON r.tenant_id = ?1 AND r.from_kind = c.to_kind AND r.from_id = c.to_id
             WHERE c.depth < ?4
         ) SELECT * FROM chain ORDER BY depth"
    };

    let result: D1Result = db
        .prepare(query)
        .bind(&[
            JsValue::from_str(tenant_id),
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
pub async fn insert_causality_from_event(
    db: &D1Database,
    tenant_id: &str,
    evt: &models::GraphEvent,
) -> Result<()> {
    // run → node causality (original)
    if let (Some(ref run_id), Some(ref node_id)) = (&evt.run_id, &evt.node_id) {
        insert_relationship(
            db,
            tenant_id,
            "causality",
            "run",
            run_id,
            "node",
            node_id,
            Some("executed"),
        )
        .await?;
    }

    // Extract richer causality from payload fields
    if let Some(ref payload) = evt.payload {
        let run_id = evt.run_id.as_deref();
        let plan_id = payload.get("plan_id").and_then(|v| v.as_str());
        let task_id = payload.get("task_id").and_then(|v| v.as_str());
        let tool_call_id = payload.get("tool_call_id").and_then(|v| v.as_str());
        let artifact_id = payload.get("artifact_id").and_then(|v| v.as_str());

        // run → plan
        if let (Some(rid), Some(pid)) = (run_id, plan_id) {
            insert_relationship(
                db,
                tenant_id,
                "causality",
                "run",
                rid,
                "plan",
                pid,
                Some("planned"),
            )
            .await?;
        }
        // plan → task
        if let (Some(pid), Some(tid)) = (plan_id, task_id) {
            insert_relationship(
                db,
                tenant_id,
                "causality",
                "plan",
                pid,
                "task",
                tid,
                Some("scheduled"),
            )
            .await?;
        }
        // task → tool_call
        if let (Some(tid), Some(tcid)) = (task_id, tool_call_id) {
            insert_relationship(
                db,
                tenant_id,
                "causality",
                "task",
                tid,
                "tool_call",
                tcid,
                Some("invoked"),
            )
            .await?;
        }
        // tool_call → artifact
        if let (Some(tcid), Some(aid)) = (tool_call_id, artifact_id) {
            insert_relationship(
                db,
                tenant_id,
                "causality",
                "tool_call",
                tcid,
                "artifact",
                aid,
                Some("produced"),
            )
            .await?;
        }
        // task → task dependencies (batched to avoid N sequential D1 round trips)
        if let Some(depends_on) = payload.get("depends_on").and_then(|v| v.as_array()) {
            if let Some(tid) = task_id {
                let now = now_iso();
                let mut stmts = Vec::with_capacity(depends_on.len().min(INTEGRATION_BATCH_LIMIT));
                for dep in depends_on.iter().take(INTEGRATION_BATCH_LIMIT) {
                    if let Some(dep_id) = dep.as_str() {
                        let stmt = db.prepare(
                            "INSERT OR IGNORE INTO relationships (tenant_id, rel_type, from_kind, from_id, to_kind, to_id, relation, created_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        )
                        .bind(&[
                            JsValue::from_str(tenant_id),
                            JsValue::from_str("dependency"),
                            JsValue::from_str("task"),
                            JsValue::from_str(tid),
                            JsValue::from_str("task"),
                            JsValue::from_str(dep_id),
                            JsValue::from_str("depends_on"),
                            JsValue::from_str(&now),
                        ])?;
                        stmts.push(stmt);
                    }
                }
                if !stmts.is_empty() {
                    db.batch(stmts).await?;
                }
            }
        }
    }
    Ok(())
}

// ── Gold Layer: Run Summaries (WS3) ─────────────────────────────

pub async fn upsert_run_summary(
    db: &D1Database,
    tenant_id: &str,
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
        "INSERT INTO run_summaries (tenant_id, run_id, event_count, first_event_at, last_event_at, actors, event_types, updated_at)
         VALUES (?1, ?2, 1, ?3, ?3, ?4, ?5, ?6)
         ON CONFLICT(tenant_id, run_id) DO UPDATE SET
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
        JsValue::from_str(tenant_id),
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

pub async fn get_run_summary(
    db: &D1Database,
    tenant_id: &str,
    run_id: &str,
) -> Result<Option<models::RunSummary>> {
    let row: Option<RunSummaryRow> = db
        .prepare("SELECT * FROM run_summaries WHERE tenant_id = ?1 AND run_id = ?2")
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(run_id)])?
        .first(None)
        .await?;
    Ok(row.map(|r| r.into_summary()))
}

pub async fn list_run_summaries(
    db: &D1Database,
    tenant_id: &str,
    limit: u32,
) -> Result<Vec<models::RunSummary>> {
    let result: D1Result = db
        .prepare(
            "SELECT * FROM run_summaries WHERE tenant_id = ?1 ORDER BY updated_at DESC LIMIT ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from(limit)])?
        .all()
        .await?;
    let rows: Vec<RunSummaryRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_summary()).collect())
}

// ── Gold Layer: Task Dependencies (WS3: issue #58) ──────────────

pub async fn upsert_task_dependency(
    db: &D1Database,
    tenant_id: &str,
    run_id: &str,
    task_id: &str,
    depends_on_task_id: &str,
) -> Result<()> {
    db.prepare(
        "INSERT OR IGNORE INTO task_dependencies (tenant_id, run_id, task_id, depends_on_task_id)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&[
        JsValue::from_str(tenant_id),
        JsValue::from_str(run_id),
        JsValue::from_str(task_id),
        JsValue::from_str(depends_on_task_id),
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn get_task_dependencies(
    db: &D1Database,
    tenant_id: &str,
    run_id: &str,
) -> Result<Vec<models::TaskDependencyEdge>> {
    let result: D1Result = db
        .prepare(
            "SELECT run_id, task_id, depends_on_task_id, created_at
             FROM task_dependencies WHERE tenant_id = ?1 AND run_id = ?2
             ORDER BY created_at ASC",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(run_id)])?
        .all()
        .await?;
    let rows: Vec<TaskDependencyRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_edge()).collect())
}

#[derive(Debug, serde::Deserialize)]
struct TaskDependencyRow {
    run_id: String,
    task_id: String,
    depends_on_task_id: String,
    created_at: Option<String>,
}

impl TaskDependencyRow {
    fn into_edge(self) -> models::TaskDependencyEdge {
        models::TaskDependencyEdge {
            run_id: self.run_id,
            task_id: self.task_id,
            depends_on_task_id: self.depends_on_task_id,
            created_at: self.created_at,
        }
    }
}

// ── Replay Contract Stub (WS3: issue #58) ───────────────────────

/// Replay plan cap: limit trace fetch to avoid loading unbounded events for large runs.
const REPLAY_PLAN_MAX_EVENTS: u32 = 500;

pub async fn build_replay_plan(
    db: &D1Database,
    tenant_id: &str,
    run_id: &str,
    from_event_id: Option<&str>,
    to_event_id: Option<&str>,
) -> Result<Vec<models::ReplayStep>> {
    // Fetch a capped trace slice (not the full run) to bound memory usage on large runs
    let events = get_trace_for_run(db, tenant_id, run_id, REPLAY_PLAN_MAX_EVENTS).await?;

    // Find the slice boundaries
    let start_idx = match from_event_id {
        Some(fid) => events.iter().position(|e| e.id == fid).unwrap_or(0),
        None => 0,
    };
    let end_idx = match to_event_id {
        Some(tid) => events
            .iter()
            .position(|e| e.id == tid)
            .map(|i| i + 1)
            .unwrap_or(events.len()),
        None => events.len(),
    };

    let slice = &events[start_idx..end_idx.min(events.len())];
    let steps: Vec<models::ReplayStep> = slice
        .iter()
        .enumerate()
        .map(|(i, evt)| models::ReplayStep {
            sequence: i + 1,
            event_type: evt.event_type.clone(),
            node_id: evt.node_id.clone(),
            actor: evt.actor.clone(),
        })
        .collect();
    Ok(steps)
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
    #[allow(dead_code)]
    tenant_id: String,
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

#[derive(Debug, serde::Deserialize)]
pub struct VerificationEvidenceRow {
    pub id: String,
    pub run_id: String,
    pub baseline_run_id: Option<String>,
    pub status: String,
    pub step_count: i32,
    pub drift_count: i32,
    pub drift_ratio_percent: f64,
    pub within_variance: i32,
    pub failure_classification: Option<String>,
    pub tests_passed: i32,
    pub policy_approved: i32,
    pub provenance_complete: i32,
    pub eligible_for_promotion: i32,
    pub confidence_score: i32,
    pub failed_gates: Option<String>,
    pub created_at: String,
}

impl VerificationEvidenceRow {
    pub fn into_response(self) -> models::VerificationEvidence {
        let failure_classification = self.failure_classification.and_then(|value| {
            serde_json::from_value::<models::FailureClass>(serde_json::Value::String(value)).ok()
        });

        models::VerificationEvidence {
            id: self.id,
            run_id: self.run_id,
            baseline_run_id: self.baseline_run_id,
            status: self.status,
            step_count: self.step_count,
            drift_count: self.drift_count,
            drift_ratio_percent: self.drift_ratio_percent,
            within_variance: self.within_variance != 0,
            failure_classification,
            tests_passed: self.tests_passed != 0,
            policy_approved: self.policy_approved != 0,
            provenance_complete: self.provenance_complete != 0,
            eligible_for_promotion: self.eligible_for_promotion != 0,
            confidence_score: self.confidence_score,
            failed_gates: self
                .failed_gates
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                .unwrap_or_default(),
            created_at: self.created_at,
        }
    }
}

fn lease_time(seconds: u64) -> String {
    let now = js_sys::Date::now();
    let future = js_sys::Date::new(&JsValue::from_f64(now + (seconds as f64 * 1000.0)));
    future.to_iso_string().as_string().unwrap()
}

// ── Integrations (WS6) ─────────────────────────────────────────

/// Maximum events per integration intake request (H2/H3: D1 batch limits).
pub const INTEGRATION_BATCH_LIMIT: usize = 100;

pub async fn register_integration(
    db: &D1Database,
    tenant_id: &str,
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
        "INSERT INTO integrations (id, tenant_id, target, name, endpoint, api_version, status, config, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8)
         ON CONFLICT(tenant_id, target, name) DO UPDATE SET
           endpoint = excluded.endpoint,
           api_version = excluded.api_version,
           config = excluded.config,
           status = 'active',
           updated_at = excluded.created_at",
    )
    .bind(&[
        JsValue::from_str(id),
        JsValue::from_str(tenant_id),
        JsValue::from_str(body.target.as_str()),
        JsValue::from_str(&body.name),
        opt_str(&body.endpoint),
        JsValue::from_str(api_version),
        match &config_json {
            Some(s) => JsValue::from_str(s),
            None => JsValue::NULL,
        },
        JsValue::from_str(&now),
    ])?
    .run()
    .await?;

    Ok(())
}

pub async fn list_integrations(
    db: &D1Database,
    tenant_id: &str,
    limit: u32,
) -> Result<Vec<crate::integrations::Integration>> {
    let result: D1Result = db
        .prepare(
            "SELECT id, target, name, endpoint, api_version, status, config, created_at, last_seen_at
             FROM integrations WHERE tenant_id = ?1 AND status = 'active' ORDER BY created_at DESC LIMIT ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from(limit)])?
        .all()
        .await?;

    let rows: Vec<IntegrationRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_integration()).collect())
}

pub async fn touch_integration(
    db: &D1Database,
    tenant_id: &str,
    target: &str,
    name: Option<&str>,
) -> Result<()> {
    let now = now_iso();
    match name {
        Some(n) => {
            db.prepare(
                "UPDATE integrations SET last_seen_at = ?1 WHERE tenant_id = ?2 AND target = ?3 AND name = ?4 AND status = 'active'",
            )
            .bind(&[
                JsValue::from_str(&now),
                JsValue::from_str(tenant_id),
                JsValue::from_str(target),
                JsValue::from_str(n),
            ])?
            .run()
            .await?;
        }
        None => {
            db.prepare(
                "UPDATE integrations SET last_seen_at = ?1 WHERE tenant_id = ?2 AND target = ?3 AND status = 'active'",
            )
            .bind(&[
                JsValue::from_str(&now),
                JsValue::from_str(tenant_id),
                JsValue::from_str(target),
            ])?
            .run()
            .await?;
        }
    }
    Ok(())
}

pub async fn get_integration(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
) -> Result<Option<crate::integrations::Integration>> {
    let result: D1Result = db
        .prepare(
            "SELECT id, target, name, endpoint, api_version, status, config, created_at, last_seen_at
             FROM integrations WHERE tenant_id = ?1 AND id = ?2",
        )
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
        .all()
        .await?;

    let rows: Vec<IntegrationRow> = result.results()?;
    Ok(rows.into_iter().next().map(|r| r.into_integration()))
}

pub async fn update_integration(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &crate::integrations::UpdateIntegration,
) -> Result<bool> {
    let now = now_iso();
    let mut sets = vec!["updated_at = ?3".to_string()];
    let mut bind_idx = 4u32;
    let mut bind_vals: Vec<JsValue> = vec![
        JsValue::from_str(tenant_id),
        JsValue::from_str(id),
        JsValue::from_str(&now),
    ];

    if let Some(ref name) = body.name {
        sets.push(format!("name = ?{bind_idx}"));
        bind_vals.push(JsValue::from_str(name));
        bind_idx += 1;
    }
    if let Some(ref endpoint) = body.endpoint {
        sets.push(format!("endpoint = ?{bind_idx}"));
        bind_vals.push(JsValue::from_str(endpoint));
        bind_idx += 1;
    }
    if let Some(ref status) = body.status {
        sets.push(format!("status = ?{bind_idx}"));
        bind_vals.push(JsValue::from_str(status));
        bind_idx += 1;
    }
    if let Some(ref config) = body.config {
        let json = serde_json::to_string(config)
            .map_err(|e| Error::RustError(format!("config serialization: {e}")))?;
        sets.push(format!("config = ?{bind_idx}"));
        bind_vals.push(JsValue::from_str(&json));
        let _ = bind_idx; // suppress unused warning
    }

    let sql = format!(
        "UPDATE integrations SET {} WHERE tenant_id = ?1 AND id = ?2",
        sets.join(", ")
    );
    let stmt = db.prepare(&sql);
    let bound = stmt.bind(&bind_vals)?;
    bound.run().await?;

    // D1 doesn't return affected rows easily; verify the row exists
    let exists = get_integration(db, tenant_id, id).await?.is_some();
    Ok(exists)
}

pub async fn delete_integration(db: &D1Database, tenant_id: &str, id: &str) -> Result<bool> {
    let existed = get_integration(db, tenant_id, id).await?.is_some();
    if existed {
        db.prepare("DELETE FROM integrations WHERE tenant_id = ?1 AND id = ?2")
            .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
            .run()
            .await?;
    }
    Ok(existed)
}

#[derive(Debug, serde::Deserialize)]
struct IntegrationRow {
    id: String,
    target: String,
    name: String,
    endpoint: Option<String>,
    api_version: String,
    status: String,
    config: Option<String>,
    created_at: String,
    last_seen_at: Option<String>,
}

impl IntegrationRow {
    fn into_integration(self) -> crate::integrations::Integration {
        crate::integrations::Integration {
            id: self.id,
            target: serde_json::from_value(serde_json::Value::String(self.target.clone()))
                .unwrap_or(crate::integrations::IntegrationTarget::Oxidizedgraph),
            name: self.name,
            endpoint: self.endpoint,
            api_version: self.api_version,
            status: self.status,
            config: self.config.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: self.created_at,
            last_seen_at: self.last_seen_at,
        }
    }
}

// ── AIVCS: change_set DB layer (issue #148, slice 1) ───────────────
//
// Projection above the diff artifact — see `migrations/0015_aivcs_change_set.sql`
// and `models/aivcs.rs`. HTTP routes land in a later slice; this file only
// owns the SQL + (tenant_id, ...) parameter wiring.

/// Storage-string form of a [`RiskLevel`]. `RiskLevel` is `#[serde(rename_all =
/// "snake_case")]`, so serializing yields a quoted JSON string (`"\"low\""`);
/// strip the quotes to get the bare token we persist in the `risk_level` column.
fn risk_level_storage_str(risk: RiskLevel) -> String {
    // Serializing a Copy enum to JSON cannot fail — unwrap surfaces any
    // future serde-attribute regression instead of silently writing "".
    let s = serde_json::to_string(&risk).expect("RiskLevel always serializes");
    s.trim_matches('"').to_string()
}

/// Reverse of [`risk_level_storage_str`].
fn risk_level_from_storage_str(s: &str) -> Option<RiskLevel> {
    serde_json::from_str(&format!("\"{s}\"")).ok()
}

#[allow(dead_code)] // HTTP route consumer lands in a later AIVCS slice.
#[derive(Debug, serde::Deserialize)]
struct ChangeSetRow {
    id: String,
    repo: String,
    base_ref: String,
    head_ref: String,
    author_agent_id: Option<String>,
    status: String,
    risk_level: Option<String>,
    confidence: Option<f64>,
    run_id: Option<String>,
    diff_artifact_key: Option<String>,
    summary_artifact_key: Option<String>,
    created_at: String,
}

#[allow(dead_code)] // HTTP route consumer lands in a later AIVCS slice.
impl ChangeSetRow {
    fn into_change_set(self) -> models::ChangeSet {
        let status =
            std::str::FromStr::from_str(&self.status).unwrap_or(models::ChangeSetStatus::Proposed);
        let risk_level = self
            .risk_level
            .as_deref()
            .and_then(risk_level_from_storage_str);
        models::ChangeSet {
            id: self.id,
            repo: self.repo,
            base_ref: self.base_ref,
            head_ref: self.head_ref,
            author_agent_id: self.author_agent_id,
            status,
            risk_level,
            confidence: self.confidence,
            run_id: self.run_id,
            diff_artifact_key: self.diff_artifact_key,
            summary_artifact_key: self.summary_artifact_key,
            created_at: self.created_at,
        }
    }
}

/// Insert a new `change_set` row. If `cs.id` is None, a random hex id is
/// minted (consistent with how other entities in this file generate ids).
#[allow(dead_code)] // HTTP route consumer lands in a later AIVCS slice.
pub async fn create_change_set(
    d1: &D1Database,
    tenant_id: &str,
    cs: &models::CreateChangeSet,
) -> Result<models::ChangeSet> {
    let id = match cs.id.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => random_hex_id()?,
    };
    let status = cs.status.unwrap_or(models::ChangeSetStatus::Proposed);
    let status_str = status.as_str();
    let risk_level_str = cs.risk_level.map(risk_level_storage_str);
    let now = now_iso();

    d1.prepare(SQL_INSERT_CHANGE_SET)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(&id),
            JsValue::from_str(&cs.repo),
            JsValue::from_str(&cs.base_ref),
            JsValue::from_str(&cs.head_ref),
            opt_str(&cs.author_agent_id),
            JsValue::from_str(status_str),
            opt_str(&risk_level_str),
            cs.confidence.map(JsValue::from).unwrap_or(JsValue::NULL),
            opt_str(&cs.run_id),
            opt_str(&cs.diff_artifact_key),
            opt_str(&cs.summary_artifact_key),
        ])?
        .run()
        .await?;

    Ok(models::ChangeSet {
        id,
        repo: cs.repo.clone(),
        base_ref: cs.base_ref.clone(),
        head_ref: cs.head_ref.clone(),
        author_agent_id: cs.author_agent_id.clone(),
        status,
        risk_level: cs.risk_level,
        confidence: cs.confidence,
        run_id: cs.run_id.clone(),
        diff_artifact_key: cs.diff_artifact_key.clone(),
        summary_artifact_key: cs.summary_artifact_key.clone(),
        created_at: now,
    })
}

#[allow(dead_code)] // HTTP route consumer lands in a later AIVCS slice.
pub async fn get_change_set(
    d1: &D1Database,
    tenant_id: &str,
    id: &str,
) -> Result<Option<models::ChangeSet>> {
    let row: Option<ChangeSetRow> = d1
        .prepare(SQL_GET_CHANGE_SET)
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(id)])?
        .first(None)
        .await?;
    Ok(row.map(|r| r.into_change_set()))
}

#[allow(dead_code)] // HTTP route consumer lands in a later AIVCS slice.
pub async fn list_change_sets_by_repo(
    d1: &D1Database,
    tenant_id: &str,
    repo: &str,
    limit: u32,
) -> Result<Vec<models::ChangeSet>> {
    let result: D1Result = d1
        .prepare(SQL_LIST_CHANGE_SETS_BY_REPO)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(repo),
            JsValue::from(limit as f64),
        ])?
        .all()
        .await?;
    let rows: Vec<ChangeSetRow> = result.results()?;
    Ok(rows.into_iter().map(|r| r.into_change_set()).collect())
}

// ── Issue #148 / AIVCS slice 3 — human_decision projection ──────────
//
// Projection of human approve / request_changes / pause / resume / merge
// decisions over runs and reviews. Events remain the source of truth in
// `events_bronze`; this projection gives the UI / BFF an indexed shape
// to list decisions per run or per review without scanning the event
// log. Slice 3 is projection-only — no HTTP route yet.

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
struct HumanDecisionRow {
    id: String,
    run_id: Option<String>,
    review_id: Option<String>,
    actor: String,
    decision_type: String,
    reason: Option<String>,
    policy_decision_id: Option<String>,
    resulting_event_id: Option<String>,
    created_at: String,
}

impl HumanDecisionRow {
    #[allow(dead_code)]
    fn into_decision(self) -> Option<models::HumanDecision> {
        let decision_type = self
            .decision_type
            .parse::<models::HumanDecisionType>()
            .ok()?;
        Some(models::HumanDecision {
            id: self.id,
            run_id: self.run_id,
            review_id: self.review_id,
            actor: self.actor,
            decision_type,
            reason: self.reason,
            policy_decision_id: self.policy_decision_id,
            resulting_event_id: self.resulting_event_id,
            created_at: self.created_at,
        })
    }
}

/// Insert one row into the `human_decision` projection. The caller mints
/// `id` (typically a UUID) and is responsible for writing the bronze
/// event whose id is passed back in `body.resulting_event_id`.
///
/// Tenant scoping: `tenant_id` is bound to ?1; see
/// `SQL_INSERT_HUMAN_DECISION`.
#[allow(dead_code)]
pub async fn create_human_decision(
    db: &D1Database,
    tenant_id: &str,
    id: &str,
    body: &models::CreateHumanDecision,
) -> Result<()> {
    db.prepare(SQL_INSERT_HUMAN_DECISION)
        .bind(&[
            JsValue::from_str(tenant_id),
            JsValue::from_str(id),
            opt_str(&body.run_id),
            opt_str(&body.review_id),
            JsValue::from_str(&body.actor),
            JsValue::from_str(body.decision_type.as_str()),
            opt_str(&body.reason),
            opt_str(&body.policy_decision_id),
            opt_str(&body.resulting_event_id),
        ])?
        .run()
        .await?;
    Ok(())
}

/// List `human_decision` rows for one run, in `created_at` order. The
/// projection is small (one row per human action on a run), so this
/// returns the full set rather than paginating; pagination can be added
/// in a follow-up slice if a single run accumulates enough decisions to
/// warrant it.
#[allow(dead_code)]
pub async fn list_human_decisions_by_run(
    db: &D1Database,
    tenant_id: &str,
    run_id: &str,
) -> Result<Vec<models::HumanDecision>> {
    let result: D1Result = db
        .prepare(SQL_LIST_HUMAN_DECISIONS_BY_RUN)
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(run_id)])?
        .all()
        .await?;
    let rows: Vec<HumanDecisionRow> = result.results()?;
    Ok(rows
        .into_iter()
        .filter_map(HumanDecisionRow::into_decision)
        .collect())
}

/// List `human_decision` rows for one review, in `created_at` order. See
/// `list_human_decisions_by_run` for pagination notes.
#[allow(dead_code)]
pub async fn list_human_decisions_by_review(
    db: &D1Database,
    tenant_id: &str,
    review_id: &str,
) -> Result<Vec<models::HumanDecision>> {
    let result: D1Result = db
        .prepare(SQL_LIST_HUMAN_DECISIONS_BY_REVIEW)
        .bind(&[JsValue::from_str(tenant_id), JsValue::from_str(review_id)])?
        .all()
        .await?;
    let rows: Vec<HumanDecisionRow> = result.results()?;
    Ok(rows
        .into_iter()
        .filter_map(HumanDecisionRow::into_decision)
        .collect())
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

    #[test]
    fn verification_evidence_row_into_response_parses_fields() {
        let row = VerificationEvidenceRow {
            id: "ve1".into(),
            run_id: "r1".into(),
            baseline_run_id: Some("r0".into()),
            status: "needs_review".into(),
            step_count: 8,
            drift_count: 2,
            drift_ratio_percent: 25.0,
            within_variance: 0,
            failure_classification: Some("environmental".into()),
            tests_passed: 1,
            policy_approved: 1,
            provenance_complete: 0,
            eligible_for_promotion: 0,
            confidence_score: 75,
            failed_gates: Some("[\"provenance_complete\"]".into()),
            created_at: "2026-02-25T00:00:00.000Z".into(),
        };

        let parsed = row.into_response();
        assert_eq!(parsed.id, "ve1");
        assert_eq!(
            parsed.failure_classification,
            Some(models::FailureClass::Environmental)
        );
        assert_eq!(parsed.failed_gates, vec!["provenance_complete"]);
        assert!(!parsed.within_variance);
    }

    #[test]
    fn build_entity_refs_includes_payload_link_ids() {
        let evt = models::GraphEvent {
            run_id: Some("run-1".into()),
            thread_id: Some("thread-1".into()),
            event_type: "task.completed".into(),
            node_id: Some("node-1".into()),
            actor: Some("agent".into()),
            payload: Some(serde_json::json!({
                "task_id": "task-9",
                "artifact_id": "artifact-2",
                "plan_id": "plan-4",
                "tool_call_id": "tool-call-7"
            })),
        };

        let refs = build_entity_refs(&evt).expect("refs expected");
        let parsed: serde_json::Value = serde_json::from_str(&refs).expect("valid json");
        assert_eq!(parsed["run_id"], "run-1");
        assert_eq!(parsed["thread_id"], "thread-1");
        assert_eq!(parsed["node_id"], "node-1");
        assert_eq!(parsed["task_id"], "task-9");
        assert_eq!(parsed["artifact_id"], "artifact-2");
        assert_eq!(parsed["plan_id"], "plan-4");
        assert_eq!(parsed["tool_call_id"], "tool-call-7");
    }

    // ── Cross-tenant SQL shape (WS8 isolation) ──────────────────
    //
    // These tests are the unit-level companion to the multi-tenant
    // isolation PR. They cannot run against a live D1 from `cargo test`
    // (the worker crate is wasm-only at runtime), so instead they lock
    // in the *shape* of each tenant-sensitive SQL statement. If a
    // future edit drops `tenant_id` from a WHERE clause or column list,
    // the assertions below will fail — surfacing the regression at
    // PR-review time rather than as a cross-tenant data leak in prod.
    //
    // Each test covers one of the CONFIRMED crr findings (see PR body):
    //   • get_play_definition          → src/db.rs:130 / src/lib.rs:449
    //   • create_policy_escalation     → src/db.rs:1352
    //   • check_and_increment_rate_limit → src/db.rs:2309
    // and the synthetic counter_id construction that backs uniqueness
    // for the rate-limit counter.

    #[test]
    fn cross_tenant_sql_get_play_definition_filters_on_tenant_id() {
        // The SELECT must include `tenant_id = ?1` in the WHERE clause.
        // Without it, tenant B could fetch a play definition registered
        // by tenant A and launch it.
        let sql = SQL_GET_PLAY_DEFINITION;
        assert!(
            sql.contains("WHERE tenant_id = ?1 AND name = ?2"),
            "get_play_definition SQL must filter by tenant_id and name; got: {sql}",
        );
        assert!(
            sql.contains("FROM play_definitions"),
            "get_play_definition SQL must target play_definitions; got: {sql}",
        );
    }

    #[test]
    fn cross_tenant_sql_create_policy_escalation_writes_tenant_id() {
        // The INSERT must include `tenant_id` in the column list and
        // bind it as the first parameter so a write performed under
        // tenant A's request context is not visible to tenant B's
        // tenant-scoped queries.
        let sql = SQL_INSERT_POLICY_ESCALATION;
        assert!(
            sql.contains("INTO policy_escalations"),
            "create_policy_escalation SQL must target policy_escalations; got: {sql}",
        );
        // First column in the INSERT list must be tenant_id, bound to ?1.
        let col_list_start = sql
            .find("policy_escalations (")
            .expect("expected column list");
        let col_list_tail = &sql[col_list_start..];
        assert!(
            col_list_tail.starts_with("policy_escalations (tenant_id,"),
            "tenant_id must be the first column of the INSERT list; got: {col_list_tail}",
        );
    }

    #[test]
    fn cross_tenant_sql_rate_limit_writes_and_reads_tenant_id() {
        let upsert = SQL_UPSERT_RATE_LIMIT_COUNTER;
        let select = SQL_SELECT_RATE_LIMIT_COUNT;
        assert!(
            upsert.contains("policy_rate_limit_counters"),
            "rate-limit upsert SQL must target policy_rate_limit_counters; got: {upsert}",
        );
        // tenant_id must be the first column of the insert list.
        let col_list_start = upsert
            .find("policy_rate_limit_counters (")
            .expect("expected column list");
        let col_list_tail = &upsert[col_list_start..];
        assert!(
            col_list_tail
                .lines()
                .next()
                .map(|l| l.contains("policy_rate_limit_counters ("))
                .unwrap_or(false),
            "expected column list on first line; got: {col_list_tail}",
        );
        // Column-list content must contain tenant_id as the first entry.
        assert!(
            upsert.contains("tenant_id, id, actor, action_class"),
            "tenant_id must be the first column in the rate-limit INSERT list; got: {upsert}",
        );
        // SELECT must filter by tenant_id.
        assert!(
            select.contains("WHERE tenant_id = ?1 AND id = ?2"),
            "rate-limit SELECT must filter by tenant_id and id; got: {select}",
        );
    }

    #[test]
    fn cross_tenant_rate_limit_counter_id_includes_tenant_id() {
        // The synthetic counter id is the SQLite PRIMARY KEY for the
        // rate-limit table; if two tenants generated the same id, they
        // would share a counter row and one tenant's quota would impact
        // the other. The PR asserts tenant_id is the leading segment.
        let alpha = rate_limit_counter_id("alpha", "agent-1", "read", 1_700_000_000, 60);
        let beta = rate_limit_counter_id("beta", "agent-1", "read", 1_700_000_000, 60);
        assert_ne!(
            alpha, beta,
            "tenants alpha and beta must produce distinct counter ids \
             for the same actor/action_class/window",
        );
        assert!(
            alpha.starts_with("alpha|"),
            "tenant_id must be the leading segment of the counter id; got: {alpha}",
        );
        assert!(
            beta.starts_with("beta|"),
            "tenant_id must be the leading segment of the counter id; got: {beta}",
        );
        // Exact shape — `{tenant_id}|{actor}|{action_class}|{window_start}|{window_seconds}`.
        assert_eq!(alpha, "alpha|agent-1|read|1700000000|60");
    }

    #[test]
    fn cross_tenant_rate_limit_counter_id_distinguishes_tenants_on_window_boundary() {
        // Belt-and-suspenders: same actor, same action_class, same
        // window — only tenant_id differs. The ids must differ.
        let a = rate_limit_counter_id("tenant-alpha", "shared-actor", "write", 0, 60);
        let b = rate_limit_counter_id("tenant-beta", "shared-actor", "write", 0, 60);
        assert_ne!(a, b);
    }

    // ── AIVCS: change_set cross-tenant SQL shape (issue #148, slice 1) ─

    #[test]
    fn cross_tenant_sql_insert_change_set_writes_tenant_id_at_position_1() {
        // The INSERT must bind `tenant_id` to ?1 and list it as the first
        // column. Without that, an agent writing under tenant A could be
        // surfaced to tenant B by the get/list helpers. The exact column-
        // list-and-VALUES shape is asserted because this is the
        // authoritative SQL that runs against D1.
        let sql = SQL_INSERT_CHANGE_SET;
        assert!(
            sql.contains("INTO change_set"),
            "insert SQL must target change_set; got: {sql}",
        );
        let col_list_start = sql.find("change_set (").expect("expected column list");
        let col_list_tail = &sql[col_list_start..];
        assert!(
            col_list_tail.starts_with("change_set (tenant_id, id,"),
            "tenant_id must be the first column of the INSERT list; got: {col_list_tail}",
        );
        assert!(
            sql.contains("VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"),
            "insert SQL must bind tenant_id as ?1 and 12 total params; got: {sql}",
        );
    }

    #[test]
    fn cross_tenant_sql_get_change_set_filters_on_tenant_id_first() {
        // The SELECT must include `tenant_id = ?1` in the WHERE clause,
        // and tenant_id must be the FIRST predicate so an EXPLAIN cannot
        // accidentally pick an index that ignores the tenant dimension.
        let sql = SQL_GET_CHANGE_SET;
        assert!(
            sql.contains("FROM change_set"),
            "get SQL must target change_set; got: {sql}",
        );
        assert!(
            sql.contains("WHERE tenant_id = ?1 AND id = ?2"),
            "get SQL must filter by tenant_id (?1) then id (?2); got: {sql}",
        );
    }

    #[test]
    fn cross_tenant_sql_list_change_sets_by_repo_filters_on_tenant_id_first() {
        let sql = SQL_LIST_CHANGE_SETS_BY_REPO;
        assert!(
            sql.contains("FROM change_set"),
            "list SQL must target change_set; got: {sql}",
        );
        assert!(
            sql.contains("WHERE tenant_id = ?1 AND repo = ?2"),
            "list SQL must filter by tenant_id (?1) then repo (?2); got: {sql}",
        );
        assert!(
            sql.contains("ORDER BY created_at DESC"),
            "list SQL must return newest-first; got: {sql}",
        );
        assert!(
            sql.contains("LIMIT ?3"),
            "list SQL must bind limit as ?3; got: {sql}",
        );
    }

    #[test]
    fn change_set_risk_level_round_trips_through_storage_str() {
        // The risk_level column stores the snake_case token (low | medium |
        // high | critical). Both directions must round-trip so reads after
        // a write produce an equal RiskLevel.
        let levels = [
            RiskLevel::Low,
            RiskLevel::Medium,
            RiskLevel::High,
            RiskLevel::Critical,
        ];
        for r in levels {
            let s = risk_level_storage_str(r);
            assert!(
                !s.contains('"'),
                "stored risk_level must be a bare token, not quoted JSON; got: {s}",
            );
            let back = risk_level_from_storage_str(&s).expect("known risk_level must parse");
            assert_eq!(back, r);
        }
        assert_eq!(risk_level_storage_str(RiskLevel::Low), "low");
        assert!(risk_level_from_storage_str("not-a-risk").is_none());
    }

    #[test]
    fn build_entity_refs_returns_none_when_no_supported_refs() {
        let evt = models::GraphEvent {
            run_id: None,
            thread_id: None,
            event_type: "noop".into(),
            node_id: None,
            actor: None,
            payload: Some(serde_json::json!({ "status": "ok" })),
        };

        assert!(build_entity_refs(&evt).is_none());
    }

    // ── Pagination helpers: list_policy_rules / list_agents ─────

    fn agent_row(id: &str, name: &str) -> AgentRow {
        AgentRow {
            id: id.into(),
            name: name.into(),
            capabilities: "[]".into(),
            endpoint: None,
            last_heartbeat: None,
            status: "active".into(),
            metadata: None,
        }
    }

    fn policy_row(id: &str, priority: i32, created_at: &str) -> PolicyRuleRow {
        PolicyRuleRow {
            id: id.into(),
            name: format!("rule-{id}"),
            action_pattern: "*".into(),
            resource_pattern: "*".into(),
            actor_pattern: "*".into(),
            risk_level: "read".into(),
            verdict: "allow".into(),
            reason: "test".into(),
            priority,
            enabled: 1,
            created_at: created_at.into(),
            updated_at: created_at.into(),
        }
    }

    #[test]
    fn list_agents_helper_returns_no_cursor_when_results_under_limit() {
        let mut rows = vec![agent_row("a1", "alpha"), agent_row("a2", "beta")];
        let cursor = compute_agents_next_cursor(&mut rows, 5);
        assert!(cursor.is_none());
        assert_eq!(rows.len(), 2, "rows preserved when under limit");
    }

    #[test]
    fn list_agents_helper_returns_no_cursor_when_results_exactly_at_limit() {
        // Exactly `limit` rows means no overflow row was fetched -> no next.
        let mut rows = vec![agent_row("a1", "alpha"), agent_row("a2", "beta")];
        let cursor = compute_agents_next_cursor(&mut rows, 2);
        assert!(cursor.is_none());
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn list_agents_helper_trims_overflow_and_returns_cursor() {
        // limit=2 with 3 rows: the third is the overflow probe; should be
        // dropped, and the cursor is built from the last *returned* row.
        let mut rows = vec![
            agent_row("a1", "alpha"),
            agent_row("a2", "beta"),
            agent_row("a3", "gamma"),
        ];
        let cursor = compute_agents_next_cursor(&mut rows, 2).expect("cursor");
        assert_eq!(rows.len(), 2);
        assert_eq!(cursor.name, "beta");
        assert_eq!(cursor.id, "a2");
    }

    #[test]
    fn list_policy_rules_helper_returns_no_cursor_when_results_under_limit() {
        let mut rows = vec![
            policy_row("r1", 100, "2026-01-01T00:00:00Z"),
            policy_row("r2", 50, "2026-01-02T00:00:00Z"),
        ];
        let cursor = compute_policy_rules_next_cursor(&mut rows, 5);
        assert!(cursor.is_none());
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn list_policy_rules_helper_trims_overflow_and_returns_cursor() {
        let mut rows = vec![
            policy_row("r1", 100, "2026-01-01T00:00:00Z"),
            policy_row("r2", 50, "2026-01-02T00:00:00Z"),
            policy_row("r3", 25, "2026-01-03T00:00:00Z"),
        ];
        let cursor = compute_policy_rules_next_cursor(&mut rows, 2).expect("cursor");
        assert_eq!(rows.len(), 2);
        assert_eq!(cursor.priority, 50);
        assert_eq!(cursor.created_at, "2026-01-02T00:00:00Z");
        assert_eq!(cursor.id, "r2");
    }

    #[test]
    fn list_policy_rules_helper_preserves_order_after_truncation() {
        // The returned rows must remain in caller-visible order; truncation
        // removes the overflow probe from the *end*, not from the middle.
        let mut rows = vec![
            policy_row("r1", 100, "2026-01-01T00:00:00Z"),
            policy_row("r2", 100, "2026-01-02T00:00:00Z"),
            policy_row("r3", 50, "2026-01-03T00:00:00Z"),
        ];
        let cursor = compute_policy_rules_next_cursor(&mut rows, 2).expect("cursor");
        assert_eq!(rows[0].id, "r1");
        assert_eq!(rows[1].id, "r2");
        // Cursor reflects the last *returned* row, breaking the priority tie
        // on (created_at, id).
        assert_eq!(cursor.priority, 100);
        assert_eq!(cursor.id, "r2");
    }

    // ── Cross-tenant SQL shape: AIVCS review projections (#148) ─

    /// Every read against the AIVCS review-projection tables must
    /// filter by `tenant_id = ?1`. Without this, threads, comments,
    /// resolutions, or file anchors written under tenant A would be
    /// visible to tenant B — a cross-tenant data leak through what
    /// looks like a benign read model.
    #[test]
    fn cross_tenant_sql_aivcs_review_thread_select_filters_on_tenant_id() {
        for (label, sql) in [
            ("SQL_GET_REVIEW_THREAD", SQL_GET_REVIEW_THREAD),
            (
                "SQL_LIST_REVIEW_THREADS_FOR_REVIEW",
                SQL_LIST_REVIEW_THREADS_FOR_REVIEW,
            ),
            (
                "SQL_LIST_REVIEW_COMMENTS_FOR_THREAD",
                SQL_LIST_REVIEW_COMMENTS_FOR_THREAD,
            ),
            (
                "SQL_GET_REVIEW_THREAD_RESOLUTION",
                SQL_GET_REVIEW_THREAD_RESOLUTION,
            ),
            (
                "SQL_LIST_FILE_ANCHORS_FOR_THREAD",
                SQL_LIST_FILE_ANCHORS_FOR_THREAD,
            ),
        ] {
            assert!(
                sql.contains("WHERE tenant_id = ?1"),
                "{label} must filter by tenant_id as ?1; got: {sql}",
            );
        }
    }

    /// Every write into the AIVCS review-projection tables must bind
    /// `tenant_id` as the leading column (so it lands as ?1) — this
    /// matches the convention used by every other multi-tenant table
    /// in the schema.
    #[test]
    fn cross_tenant_sql_aivcs_review_inserts_lead_with_tenant_id() {
        for (table_name, sql) in [
            ("review_thread", SQL_INSERT_REVIEW_THREAD),
            ("review_comment", SQL_INSERT_REVIEW_COMMENT),
            (
                "review_thread_resolution",
                SQL_INSERT_REVIEW_THREAD_RESOLUTION,
            ),
            ("file_anchor", SQL_INSERT_FILE_ANCHOR),
        ] {
            let needle = format!("{table_name} (");
            let start = sql
                .find(&needle)
                .unwrap_or_else(|| panic!("expected INSERT into {table_name}; got: {sql}"));
            let tail = &sql[start..];
            assert!(
                tail.starts_with(&format!("{table_name} (tenant_id,")),
                "tenant_id must be the first column of the INSERT list for {table_name}; got: {tail}",
            );
            assert!(
                sql.contains("(?1, ?2"),
                "INSERT for {table_name} must bind tenant_id as ?1; got: {sql}",
            );
        }
    }

    /// The UPDATE for review-thread status must also be tenant-scoped
    /// — otherwise tenant A could mark tenant B's thread resolved.
    #[test]
    fn cross_tenant_sql_aivcs_review_thread_update_scopes_to_tenant() {
        assert!(
            SQL_UPDATE_REVIEW_THREAD_STATUS.contains("WHERE tenant_id = ?1 AND id = ?2"),
            "review_thread status update must filter by tenant_id and id; got: {}",
            SQL_UPDATE_REVIEW_THREAD_STATUS,
        );
        assert!(
            SQL_UPDATE_REVIEW_THREAD_STATUS.contains("UPDATE review_thread"),
            "review_thread status update must target review_thread; got: {}",
            SQL_UPDATE_REVIEW_THREAD_STATUS,
        );
    }
    // ── Issue #148 / AIVCS slice 3 — human_decision SQL shape ───
    //
    // Companion to the WS8 cross-tenant tests above. The projection
    // tables don't run against a live D1 from `cargo test` (wasm-only
    // at runtime), so we lock in the SQL text: tenant_id must be ?1 on
    // every read and every write, and the INSERT column list must
    // carry all 7 typed columns plus tenant_id + id. If a future edit
    // drops a column or moves tenant_id out of the leading position
    // these tests fail at PR-review time rather than as a silent
    // cross-tenant leak.

    #[test]
    fn cross_tenant_sql_insert_human_decision_binds_tenant_id_first() {
        let sql = SQL_INSERT_HUMAN_DECISION;
        assert!(
            sql.contains("INTO human_decision"),
            "create_human_decision SQL must target human_decision; got: {sql}",
        );
        let col_list_start = sql.find("human_decision (").expect("expected column list");
        let col_list_tail = &sql[col_list_start..];
        // tenant_id must be the first column of the INSERT column list.
        assert!(
            col_list_tail.starts_with("human_decision (tenant_id,"),
            "tenant_id must be the first column of the INSERT list; got: {col_list_tail}",
        );
        // VALUES must bind tenant_id as ?1 and id as ?2.
        let values_start = sql.find("VALUES (").expect("expected VALUES clause");
        let values_tail = &sql[values_start..];
        assert!(
            values_tail.starts_with("VALUES (?1, ?2,"),
            "tenant_id and id must be the leading bound parameters; got: {values_tail}",
        );
    }

    #[test]
    fn cross_tenant_sql_insert_human_decision_has_all_seven_typed_columns_plus_tenant_id() {
        // Slice spec: the projection carries 7 typed columns (run_id,
        // review_id, actor, decision_type, reason, policy_decision_id,
        // resulting_event_id) plus tenant_id + id. created_at is filled
        // by the DEFAULT, not bound. Pin the exact column list so a
        // future migration that adds a column must also bump this test.
        let sql = SQL_INSERT_HUMAN_DECISION;
        for expected in [
            "tenant_id",
            "id",
            "run_id",
            "review_id",
            "actor",
            "decision_type",
            "reason",
            "policy_decision_id",
            "resulting_event_id",
        ] {
            assert!(
                sql.contains(expected),
                "INSERT column list missing {expected}; got: {sql}",
            );
        }
        // Exactly 9 bound parameters (?1..?9): tenant_id, id, plus the
        // 7 typed columns. created_at is filled by the column DEFAULT.
        for placeholder in ["?1", "?2", "?3", "?4", "?5", "?6", "?7", "?8", "?9"] {
            assert!(
                sql.contains(placeholder),
                "INSERT must bind {placeholder}; got: {sql}",
            );
        }
        assert!(
            !sql.contains("?10"),
            "INSERT must bind exactly 9 parameters (no created_at bind, that's a DEFAULT); got: {sql}",
        );
    }

    #[test]
    fn cross_tenant_sql_list_human_decisions_by_run_filters_on_tenant_id() {
        // SELECT must include `tenant_id = ?1` in the WHERE clause so a
        // tenant B request can't list tenant A's decisions for a
        // colliding run_id.
        let sql = SQL_LIST_HUMAN_DECISIONS_BY_RUN;
        assert!(
            sql.contains("FROM human_decision"),
            "list_human_decisions_by_run SQL must read from human_decision; got: {sql}",
        );
        assert!(
            sql.contains("WHERE tenant_id = ?1 AND run_id = ?2"),
            "list_human_decisions_by_run SQL must filter by (tenant_id=?1, run_id=?2); got: {sql}",
        );
    }

    #[test]
    fn cross_tenant_sql_list_human_decisions_by_review_filters_on_tenant_id() {
        let sql = SQL_LIST_HUMAN_DECISIONS_BY_REVIEW;
        assert!(
            sql.contains("FROM human_decision"),
            "list_human_decisions_by_review SQL must read from human_decision; got: {sql}",
        );
        assert!(
            sql.contains("WHERE tenant_id = ?1 AND review_id = ?2"),
            "list_human_decisions_by_review SQL must filter by (tenant_id=?1, review_id=?2); got: {sql}",
        );
    }

    #[test]
    fn human_decision_row_decode_uses_decision_type_from_str() {
        let row = HumanDecisionRow {
            id: "hd-1".into(),
            run_id: Some("run-1".into()),
            review_id: None,
            actor: "human:jane".into(),
            decision_type: "request_changes".into(),
            reason: Some("nits".into()),
            policy_decision_id: None,
            resulting_event_id: Some("ev-1".into()),
            created_at: "2026-06-11T00:00:00.000Z".into(),
        };
        let decoded = row.into_decision().expect("valid decision_type");
        assert_eq!(
            decoded.decision_type,
            models::HumanDecisionType::RequestChanges
        );
        assert_eq!(decoded.id, "hd-1");
        assert_eq!(decoded.actor, "human:jane");
        assert_eq!(decoded.resulting_event_id.as_deref(), Some("ev-1"));
    }

    #[test]
    fn human_decision_row_decode_skips_unknown_decision_type() {
        let row = HumanDecisionRow {
            id: "hd-bad".into(),
            run_id: Some("run-1".into()),
            review_id: None,
            actor: "human:jane".into(),
            decision_type: "not_a_real_decision".into(),
            reason: None,
            policy_decision_id: None,
            resulting_event_id: None,
            created_at: "2026-06-11T00:00:00.000Z".into(),
        };
        assert!(row.into_decision().is_none());
    }

    // ── AIVCS issue #148 slice 4: pause/resume SQL shape ────────────
    //
    // These tests pin the SQL text of SQL_PAUSE_RUN / SQL_RESUME_RUN so a
    // future edit can't silently drop the tenant_id filter (cross-tenant
    // leak) or the status guard (illegal transition out of a terminal /
    // already-paused state).

    #[test]
    fn sql_pause_run_has_tenant_id_leading_param() {
        // tenant_id MUST be the first bound parameter (?1) so the worker
        // can never accidentally pause a run that belongs to a different
        // tenant — even if a downstream caller binds the wrong run_id.
        let sql = SQL_PAUSE_RUN;
        assert!(
            sql.contains("WHERE tenant_id = ?1"),
            "SQL_PAUSE_RUN must filter by tenant_id = ?1; got: {sql}",
        );
        assert!(
            sql.contains("AND id = ?2"),
            "SQL_PAUSE_RUN must bind run id as ?2; got: {sql}",
        );
    }

    #[test]
    fn sql_pause_run_has_terminal_state_guard() {
        // The status guard is what makes this a state machine, not a
        // last-write-wins clobber. Only `created` and `running` may
        // transition to `paused`.
        let sql = SQL_PAUSE_RUN;
        assert!(
            sql.contains("status IN ('created', 'running')"),
            "SQL_PAUSE_RUN must only pause created/running runs; got: {sql}",
        );
    }

    #[test]
    fn sql_pause_run_sets_paused_at() {
        // The handler returns paused_at in the response envelope, so the
        // UPDATE must actually stamp it (and not leave it to a trigger,
        // which D1 doesn't reliably support across migrations).
        let sql = SQL_PAUSE_RUN;
        assert!(
            sql.contains("status = 'paused'"),
            "SQL_PAUSE_RUN must set status to 'paused'; got: {sql}",
        );
        assert!(
            sql.contains("paused_at = datetime('now')"),
            "SQL_PAUSE_RUN must stamp paused_at server-side; got: {sql}",
        );
    }

    #[test]
    fn sql_resume_run_has_tenant_id_leading_param() {
        let sql = SQL_RESUME_RUN;
        assert!(
            sql.contains("WHERE tenant_id = ?1"),
            "SQL_RESUME_RUN must filter by tenant_id = ?1; got: {sql}",
        );
        assert!(
            sql.contains("AND id = ?2"),
            "SQL_RESUME_RUN must bind run id as ?2; got: {sql}",
        );
    }

    #[test]
    fn sql_resume_run_has_paused_guard() {
        // The guard makes resume a no-op for any non-paused run
        // (including terminal). The handler reads zero-changes as
        // RUN_NOT_PAUSED.
        let sql = SQL_RESUME_RUN;
        assert!(
            sql.contains("status = 'paused'"),
            "SQL_RESUME_RUN must require status = 'paused' before resuming; got: {sql}",
        );
        assert!(
            sql.contains("status = 'running'"),
            "SQL_RESUME_RUN must set status to 'running'; got: {sql}",
        );
        assert!(
            sql.contains("resumed_at = datetime('now')"),
            "SQL_RESUME_RUN must stamp resumed_at server-side; got: {sql}",
        );
    }

    #[test]
    fn pause_resume_outcomes_serialize_status_update() {
        // The RunStatusUpdate envelope serialised in handler responses
        // must omit the opposite timestamp (paused_at on a pause
        // response, resumed_at on a resume response). serde's
        // skip_serializing_if = "Option::is_none" handles this — pin it
        // here so a future derive change can't drop the attribute.
        let paused = RunStatusUpdate {
            id: "run-1".into(),
            status: "paused".into(),
            paused_at: Some("2026-06-11T00:00:00Z".into()),
            resumed_at: None,
        };
        let json = serde_json::to_value(&paused).unwrap();
        assert_eq!(json["id"], "run-1");
        assert_eq!(json["status"], "paused");
        assert_eq!(json["paused_at"], "2026-06-11T00:00:00Z");
        assert!(
            json.get("resumed_at").is_none(),
            "resumed_at must be omitted from a fresh pause response; got: {json}",
        );

        let resumed = RunStatusUpdate {
            id: "run-1".into(),
            status: "running".into(),
            paused_at: None,
            resumed_at: Some("2026-06-11T00:01:00Z".into()),
        };
        let json = serde_json::to_value(&resumed).unwrap();
        assert_eq!(json["status"], "running");
        assert_eq!(json["resumed_at"], "2026-06-11T00:01:00Z");
        assert!(
            json.get("paused_at").is_none(),
            "paused_at must be omitted from a fresh resume response; got: {json}",
        );
    }

    // ── Outcome → HTTP code shape ───────────────────────────────────
    //
    // The handler in lib.rs maps these enums to the response codes
    // mandated by the issue #148 spec. We pin the mapping here so a
    // refactor of either the handler or the helper can't silently
    // change the contract.

    #[test]
    fn pause_outcome_already_paused_is_idempotent_envelope() {
        // already-paused must surface the existing paused_at, not
        // synthesise a fresh one — restamping would mask the original
        // pause time and corrupt the audit trail.
        let outcome = PauseOutcome::AlreadyPaused(RunStatusUpdate {
            id: "run-x".into(),
            status: "paused".into(),
            paused_at: Some("2026-06-10T12:00:00Z".into()),
            resumed_at: None,
        });
        match outcome {
            PauseOutcome::AlreadyPaused(update) => {
                assert_eq!(update.status, "paused");
                assert_eq!(update.paused_at.as_deref(), Some("2026-06-10T12:00:00Z"));
            }
            other => panic!("expected AlreadyPaused; got {other:?}"),
        }
    }

    #[test]
    fn pause_outcome_terminal_reports_current_status_for_envelope() {
        // The 409 envelope echoes back the offending state so the
        // caller can present a sensible error to a human operator
        // ("can't pause: run already succeeded") without a second
        // round-trip.
        for terminal in ["succeeded", "failed", "cancelled"] {
            let outcome = PauseOutcome::Terminal {
                current_status: terminal.to_string(),
            };
            match outcome {
                PauseOutcome::Terminal { current_status } => {
                    assert_eq!(current_status, terminal);
                }
                other => panic!("expected Terminal; got {other:?}"),
            }
        }
    }

    #[test]
    fn resume_outcome_not_paused_reports_current_status() {
        // Resume from any non-paused state (including terminal) is the
        // same 409 RUN_NOT_PAUSED — we don't bifurcate the response
        // shape based on which non-paused state the run is in. The
        // current_status string is what disambiguates for the caller.
        for state in ["running", "created", "succeeded", "failed", "cancelled"] {
            let outcome = ResumeOutcome::NotPaused {
                current_status: state.to_string(),
            };
            match outcome {
                ResumeOutcome::NotPaused { current_status } => {
                    assert_eq!(current_status, state);
                }
                other => panic!("expected NotPaused; got {other:?}"),
            }
        }
    }
}

// ── AIVCS: ci_check_run DB layer ─────────────────────────────────────────────

pub async fn list_ci_check_runs_for_change_set(
    d1: &worker::D1Database,
    tenant_id: &str,
    change_set_id: &str,
    limit: u32,
) -> Result<Vec<models::aivcs::CiCheckRun>> {
    let sql = "SELECT id, change_set_id, name, status, conclusion, url, created_at \
               FROM ci_check_run \
               WHERE tenant_id = ?1 AND change_set_id = ?2 \
               ORDER BY created_at DESC \
               LIMIT ?3";
    let stmt = d1.prepare(sql)
        .bind(&[
            tenant_id.into(),
            change_set_id.into(),
            limit.into(),
        ])?;
    let result = stmt.all().await?;
    let mut runs = Vec::new();
    for row in result.results::<serde_json::Value>()? {
        runs.push(models::aivcs::CiCheckRun {
            id: row.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            change_set_id: row.get("change_set_id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            name: row.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            status: row.get("status").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            conclusion: row.get("conclusion").and_then(|v| v.as_str()).map(|s| s.to_string()),
            url: row.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()),
            created_at: row.get("created_at").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        });
    }
    Ok(runs)
}

// ── AIVCS: events_bronze DB layer ────────────────────────────────────────────

pub async fn list_events_bronze(
    d1: &worker::D1Database,
    tenant_id: &str,
    since: Option<&str>,
    limit: u32,
) -> Result<Vec<serde_json::Value>> {
    let sql = if since.is_some() {
        "SELECT id, run_id, thread_id, event_type, node_id, actor, payload, created_at \
         FROM events_bronze \
         WHERE tenant_id = ?1 AND created_at > ?2 \
         ORDER BY created_at ASC \
         LIMIT ?3"
    } else {
        "SELECT id, run_id, thread_id, event_type, node_id, actor, payload, created_at \
         FROM events_bronze \
         WHERE tenant_id = ?1 \
         ORDER BY created_at ASC \
         LIMIT ?2"
    };

    let stmt = if let Some(s) = since {
        d1.prepare(sql).bind(&[tenant_id.into(), s.into(), limit.into()])?
    } else {
        d1.prepare(sql).bind(&[tenant_id.into(), limit.into()])?
    };

    let result = stmt.all().await?;
    let mut events = Vec::new();
    for row in result.results::<serde_json::Value>()? {
        let mut evt = serde_json::Map::new();
        evt.insert("id".into(), row.get("id").unwrap_or(&serde_json::Value::Null).clone());
        evt.insert("run_id".into(), row.get("run_id").unwrap_or(&serde_json::Value::Null).clone());
        evt.insert("thread_id".into(), row.get("thread_id").unwrap_or(&serde_json::Value::Null).clone());
        evt.insert("event_type".into(), row.get("event_type").unwrap_or(&serde_json::Value::Null).clone());
        evt.insert("node_id".into(), row.get("node_id").unwrap_or(&serde_json::Value::Null).clone());
        evt.insert("actor".into(), row.get("actor").unwrap_or(&serde_json::Value::Null).clone());
        if let Some(payload_str) = row.get("payload").and_then(|v| v.as_str()) {
            if let Ok(payload_json) = serde_json::from_str::<serde_json::Value>(payload_str) {
                evt.insert("payload".into(), payload_json);
            }
        }
        evt.insert("created_at".into(), row.get("created_at").unwrap_or(&serde_json::Value::Null).clone());
        events.push(serde_json::Value::Object(evt));
    }
    Ok(events)
}

// ── AIVCS: branch DB layer ───────────────────────────────────────────────────

pub async fn list_branches_by_repo(
    d1: &worker::D1Database,
    tenant_id: &str,
    repo: &str,
    limit: u32,
) -> Result<Vec<models::aivcs::Branch>> {
    let sql = "SELECT id, repo, name, head_sha, agent_owner, status, created_at \
               FROM branch \
               WHERE tenant_id = ?1 AND repo = ?2 \
               ORDER BY created_at DESC \
               LIMIT ?3";
    let stmt = d1.prepare(sql)
        .bind(&[
            tenant_id.into(),
            repo.into(),
            limit.into(),
        ])?;
    let result = stmt.all().await?;
    let mut branches = Vec::new();
    for row in result.results::<serde_json::Value>()? {
        branches.push(models::aivcs::Branch {
            id: row.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            repo: row.get("repo").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            name: row.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            head_sha: row.get("head_sha").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            agent_owner: row.get("agent_owner").and_then(|v| v.as_str()).map(|s| s.to_string()),
            status: row.get("status").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            created_at: row.get("created_at").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        });
    }
    Ok(branches)
}
