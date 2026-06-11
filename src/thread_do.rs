use crate::models::{Checkpoint, CreateCheckpoint};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use worker::*;

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug)]
struct ThreadState {
    latest_checkpoint: Option<Checkpoint>,
    history: VecDeque<Checkpoint>,
}

/// Maximum number of checkpoints retained in the in-DO history ring buffer.
/// Older checkpoints fall off the back when this cap is hit.
pub(crate) const HISTORY_CAP: usize = 50;

/// Hard upper bound for inline checkpoint state stored directly inside the DO.
/// Payloads larger than this should be stashed in R2 by the caller; the DO
/// handler currently only documents this in a comment (see
/// src/thread_do.rs:50). The size-validation helper is exposed so future
/// production code can enforce it without further refactor.
#[allow(dead_code)]
pub(crate) const MAX_STATE_BYTES: usize = 128 * 1024;

// ── Pure state-machine helpers ──────────────────────────────────────────
//
// These free functions exist so the state-machine logic can be unit-tested
// natively. The production handler still inlines the same logic; PR C will
// finish the refactor. Until then they're `#[allow(dead_code)]` to keep
// `-D warnings` clean on the wasm target.

#[allow(dead_code)]
/// Append a checkpoint to the front of `history`, trimming the oldest entry
/// when the cap is exceeded. Mirrors the body of the `/checkpoint` handler.
pub(crate) fn append_to_history(history: &mut VecDeque<Checkpoint>, cp: Checkpoint) {
    history.push_front(cp);
    while history.len() > HISTORY_CAP {
        history.pop_back();
    }
}

#[allow(dead_code)]
/// Serialize `state` to JSON and return its byte length.
pub(crate) fn measured_state_size(state: &serde_json::Value) -> usize {
    serde_json::to_string(state).map(|s| s.len()).unwrap_or(0)
}

#[allow(dead_code)]
/// Returns `Ok(size)` if `state` fits within `MAX_STATE_BYTES`, else `Err(size)`.
/// This is what a future PR (C) will wire into the `/checkpoint` handler so
/// over-cap states are rejected rather than silently stored.
pub(crate) fn validate_state_size(state: &serde_json::Value) -> std::result::Result<usize, usize> {
    let size = measured_state_size(state);
    if size > MAX_STATE_BYTES {
        Err(size)
    } else {
        Ok(size)
    }
}

#[durable_object]
pub struct ThreadManager {
    state: State,
    #[allow(dead_code)]
    env: Env,
}

impl DurableObject for ThreadManager {
    fn new(state: State, env: Env) -> Self {
        Self { state, env }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let path = req.path();
        let method = req.method();

        match (method, path.as_str()) {
            (Method::Post, "/checkpoint") => {
                let body: CreateCheckpoint = req.json().await?;
                let storage = self.state.storage();

                let id = crate::generate_id().unwrap_or_else(|_| "err".to_string());
                let now = js_sys::Date::now() as u64;
                let created_at = js_sys::Date::new(&serde_wasm_bindgen::to_value(&now).unwrap()).to_iso_string().as_string().unwrap();

                let checkpoint = Checkpoint {
                    id: id.clone(),
                    thread_id: body.thread_id.clone(),
                    node_id: body.node_id.clone(),
                    parent_id: body.parent_id.clone(),
                    state_r2_key: format!("threads/{}/{}", body.thread_id, id), // We'll still use R2 for large blobs if needed, but DO can store small states
                    state_size_bytes: Some(serde_json::to_string(&body.state).unwrap_or_default().len() as i64),
                    metadata: body.metadata.clone(),
                    created_at,
                };

                // Store state in DO storage directly for fast access
                // If state is too large (> 128KB), we should probably fail or use R2
                storage.put(&format!("state:{}", id), &body.state).await?;
                storage.put("latest", &checkpoint).await?;

                let mut history: VecDeque<Checkpoint> = storage.get("history").await.ok().flatten().unwrap_or_default();
                history.push_front(checkpoint.clone());
                if history.len() > HISTORY_CAP {
                    history.pop_back();
                }
                storage.put("history", history).await?;

                Response::from_json(&checkpoint)
            }
            (Method::Get, "/latest") => {
                let storage = self.state.storage();
                let latest: Option<Checkpoint> = storage.get("latest").await.ok().flatten();

                if let Some(cp) = latest {
                    let state: Option<serde_json::Value> = storage.get(&format!("state:{}", cp.id)).await.ok().flatten();
                    // We need a way to return the state too.
                    // Let's wrap it in a response that includes the state.
                    Response::from_json(&serde_json::json!({
                        "checkpoint": cp,
                        "state": state
                    }))
                } else {
                    Ok(Response::empty()?.with_status(404))
                }
            }
            (Method::Get, "/history") => {
                let storage = self.state.storage();
                let history: VecDeque<Checkpoint> = storage.get("history").await.ok().flatten().unwrap_or_default();
                Response::from_json(&history)
            }
            _ => Response::error("not found", 404),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pure state-machine helpers extracted from the
    //! ThreadManager DO. These do not exercise the actual `storage` layer.

    use super::*;
    use crate::models::Checkpoint;

    fn make_cp(id: &str) -> Checkpoint {
        Checkpoint {
            id: id.to_string(),
            thread_id: "thread-1".to_string(),
            node_id: "node-1".to_string(),
            parent_id: None,
            state_r2_key: format!("threads/thread-1/{}", id),
            state_size_bytes: Some(0),
            metadata: None,
            created_at: format!("ts-{}", id),
        }
    }

    // ── append + history order ─────────────────────────────────────

    #[test]
    fn append_checkpoint_then_history_returns_expected_order() {
        let mut history: VecDeque<Checkpoint> = VecDeque::new();
        append_to_history(&mut history, make_cp("a"));
        append_to_history(&mut history, make_cp("b"));
        append_to_history(&mut history, make_cp("c"));

        // `push_front` means the newest is at index 0 — this matches the
        // existing handler's behaviour and is what `/history` returns.
        let ids: Vec<_> = history.iter().map(|cp| cp.id.clone()).collect();
        assert_eq!(ids, vec!["c", "b", "a"]);
    }

    // ── history cap (50) ───────────────────────────────────────────

    #[test]
    fn history_is_trimmed_at_documented_cap() {
        let mut history: VecDeque<Checkpoint> = VecDeque::new();
        // Insert HISTORY_CAP + 5 checkpoints — verify the cap holds.
        for i in 0..(HISTORY_CAP + 5) {
            append_to_history(&mut history, make_cp(&format!("cp-{i:03}")));
        }

        assert_eq!(history.len(), HISTORY_CAP);
        // First-in (cp-000 ... cp-004) must have been evicted.
        let ids: Vec<&str> = history.iter().map(|cp| cp.id.as_str()).collect();
        assert!(!ids.contains(&"cp-000"), "oldest entry should have been popped");
        assert!(!ids.contains(&"cp-004"), "first 5 should have been popped");
        // Newest is at front.
        assert_eq!(history.front().unwrap().id, format!("cp-{:03}", HISTORY_CAP + 4));
    }

    #[test]
    fn history_first_in_is_dropped_on_overflow() {
        let mut history: VecDeque<Checkpoint> = VecDeque::new();
        // Fill exactly to cap.
        for i in 0..HISTORY_CAP {
            append_to_history(&mut history, make_cp(&format!("cp-{i:03}")));
        }
        assert_eq!(history.len(), HISTORY_CAP);
        let oldest_id = history.back().unwrap().id.clone();
        assert_eq!(oldest_id, "cp-000");

        // One more push must drop the FIFO-oldest.
        append_to_history(&mut history, make_cp("overflow"));
        assert_eq!(history.len(), HISTORY_CAP);
        assert!(
            history.iter().all(|cp| cp.id != "cp-000"),
            "first-in entry must have been dropped"
        );
        assert_eq!(history.front().unwrap().id, "overflow");
    }

    // ── state-size validation (128 KB) ─────────────────────────────

    #[test]
    fn checkpoint_at_boundary_size_is_accepted() {
        // Build a JSON value that serializes to exactly MAX_STATE_BYTES.
        // A JSON string of N raw bytes serializes to N + 2 chars (the
        // surrounding quotes), so we use a string of length MAX - 2 and
        // assert the boundary case is accepted.
        let payload: String = "a".repeat(MAX_STATE_BYTES - 2);
        let state = serde_json::Value::String(payload);
        let sz = measured_state_size(&state);
        assert_eq!(sz, MAX_STATE_BYTES, "boundary fixture should land at cap");
        assert!(validate_state_size(&state).is_ok());
    }

    #[test]
    fn checkpoint_over_cap_is_rejected() {
        let payload: String = "b".repeat(MAX_STATE_BYTES); // serializes to MAX+2
        let state = serde_json::Value::String(payload);
        let sz = measured_state_size(&state);
        assert!(sz > MAX_STATE_BYTES);

        let res = validate_state_size(&state);
        assert!(res.is_err(), "state larger than cap must be rejected");
        assert_eq!(res.unwrap_err(), sz);
    }

    // ── PR C follow-up: state-key cleanup after trim ───────────────
    //
    // The PR brief notes that once PR C lands and the trim step also
    // cleans up `state:{id}` keys for evicted checkpoints, we should
    // assert no `state:{id}` keys remain for trimmed entries. That
    // cleanup is not yet implemented (the handler still calls
    // `storage.put("state:{id}", ...)` but never deletes), so the
    // assertion is deferred. See PR body "Out of scope" and follow-up.
    #[test]
    fn pr_c_follow_up_state_key_cleanup_is_not_yet_implemented() {
        // Documentation-only test that pins this gap so future PR C
        // contributors find it during their refactor.
        // No assertion — we just want the gap to surface in test output
        // if anyone ever tries to claim "trim cleans state keys".
        let mut history: VecDeque<Checkpoint> = VecDeque::new();
        for i in 0..(HISTORY_CAP + 1) {
            append_to_history(&mut history, make_cp(&format!("cp-{i}")));
        }
        // We can only verify the in-memory ring-buffer here; the
        // state-key cleanup lives behind real DO storage and is out
        // of scope for this PR.
        assert_eq!(history.len(), HISTORY_CAP);
    }
}
