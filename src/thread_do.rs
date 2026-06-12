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

/// Maximum serialized state size accepted by the ThreadManager DO. Above
/// this the caller must spill to R2 (the slow-path `/v1/checkpoints`
/// handler in `lib.rs` already writes the canonical copy to R2; the DO
/// is only a hot-cache for small states). Pre-fix the DO accepted
/// arbitrarily-large states and never reclaimed `state:{id}` keys when
/// history was trimmed — see PR #132 crr finding on thread_do.rs:44.
const MAX_DO_STATE_BYTES: usize = 128 * 1024;

/// Maximum number of checkpoints kept in the in-DO history ring. When
/// the ring overflows the oldest entry is evicted AND its associated
/// `state:{id}` storage key is deleted to prevent the storage leak
/// previously seen at thread_do.rs:57.
const MAX_HISTORY_ENTRIES: usize = 50;

/// Decision returned by `validate_checkpoint_size`. Used by the
/// `/checkpoint` handler to choose between accept and 413-reject. Kept
/// as a tiny enum (rather than just `Result<()>`) so the helper can be
/// unit-tested for both branches with a single round-trip.
#[derive(Debug, PartialEq)]
pub(crate) enum CheckpointSize {
    Ok(usize),
    /// State exceeds `MAX_DO_STATE_BYTES`. Carries the observed size so
    /// the error response can quote it to the caller.
    TooLarge {
        observed: usize,
        limit: usize,
    },
}

/// Pure helper: classify a serialized state payload by size against the
/// 128 KB DO limit. Extracted so it can be unit-tested without a
/// Workers runtime (see `mod tests`).
pub(crate) fn validate_checkpoint_size(serialized_len: usize) -> CheckpointSize {
    if serialized_len > MAX_DO_STATE_BYTES {
        CheckpointSize::TooLarge {
            observed: serialized_len,
            limit: MAX_DO_STATE_BYTES,
        }
    } else {
        CheckpointSize::Ok(serialized_len)
    }
}

/// Pure helper: given the current history ring (newest-first) and a new
/// checkpoint pushed to the front, return the list of checkpoint ids
/// that were evicted because the ring overflowed.
///
/// The DO uses this output to call `storage.delete("state:{id}")` for
/// each evicted id — fixing the storage leak called out at
/// thread_do.rs:57 (history trim never cleaned up the `state:{id}`
/// keys, so they accumulated indefinitely).
pub(crate) fn ids_to_evict_for_trim(history: &VecDeque<Checkpoint>, max: usize) -> Vec<String> {
    if history.len() <= max {
        return Vec::new();
    }
    // Eviction policy: drop oldest-first (back of the deque), which
    // matches the existing `history.pop_back()` call.
    let n_to_drop = history.len() - max;
    history
        .iter()
        .rev()
        .take(n_to_drop)
        .map(|cp| cp.id.clone())
        .collect()
}

/// Append a checkpoint to the front of `history`, trimming the oldest entry
/// when the cap is exceeded. Mirrors the body of the `/checkpoint` handler.
#[allow(dead_code)]
pub(crate) fn append_to_history(history: &mut VecDeque<Checkpoint>, cp: Checkpoint, max: usize) {
    history.push_front(cp);
    while history.len() > max {
        history.pop_back();
    }
}

/// Serialize `state` to JSON and return its byte length.
#[allow(dead_code)]
pub(crate) fn measured_state_size(state: &serde_json::Value) -> usize {
    serde_json::to_string(state).map(|s| s.len()).unwrap_or(0)
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

                let id = req
                    .headers()
                    .get("x-checkpoint-id")?
                    .ok_or_else(|| Error::RustError("missing x-checkpoint-id header".into()))?;
                let now = js_sys::Date::now() as u64;
                let created_at = js_sys::Date::new(&serde_wasm_bindgen::to_value(&now).unwrap())
                    .to_iso_string()
                    .as_string()
                    .unwrap();

                // PR #132 crr finding (thread_do.rs:44/50): the size was
                // measured but never compared against the 128 KB limit,
                // so large states landed in DO storage uncapped and
                // could blow past the per-class DO storage budget. We
                // now reject with 413 + a clear message — R2 spill is a
                // separate follow-up (this PR's body documents the
                // deferral).
                let serialized = serde_json::to_string(&body.state).unwrap_or_default();
                let state_size_bytes = serialized.len();
                if let CheckpointSize::TooLarge { observed, limit } =
                    validate_checkpoint_size(state_size_bytes)
                {
                    let msg = format!(
                        "checkpoint state {} bytes exceeds DO limit {} bytes; \
                         large checkpoints must spill to R2 (R2 spill is a \
                         follow-up; for now the slow-path /v1/checkpoints \
                         handler writes the canonical R2 copy and the DO \
                         is hot-cache only)",
                        observed, limit
                    );
                    return Response::error(msg, 413);
                }

                let checkpoint = Checkpoint {
                    id: id.clone(),
                    thread_id: body.thread_id.clone(),
                    node_id: body.node_id.clone(),
                    parent_id: body.parent_id.clone(),
                    // We still record the R2 key shape so downstream
                    // readers can fall back to R2 if the DO entry was
                    // ever evicted out-of-band.
                    state_r2_key: format!("threads/{}/{}", body.thread_id, id),
                    state_size_bytes: Some(state_size_bytes as i64),
                    metadata: body.metadata.clone(),
                    created_at,
                };

                storage.put(&format!("state:{}", id), &body.state).await?;
                storage.put("latest", &checkpoint).await?;

                let mut history: VecDeque<Checkpoint> = storage
                    .get("history")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                history.push_front(checkpoint.clone());

                // PR #132 crr finding (thread_do.rs:57): the old code
                // dropped the oldest history entry but never deleted the
                // corresponding `state:{id}` key — permanent storage
                // leak. Now we collect the evicted ids first, trim the
                // ring, persist history, then delete the stranded state
                // keys.
                let evicted_ids = ids_to_evict_for_trim(&history, MAX_HISTORY_ENTRIES);
                while history.len() > MAX_HISTORY_ENTRIES {
                    history.pop_back();
                }
                storage.put("history", history).await?;

                for ev_id in evicted_ids {
                    if let Err(e) = storage.delete(&format!("state:{}", ev_id)).await {
                        // Best-effort: a delete failure is logged but
                        // does not fail the whole checkpoint write.
                        worker::console_log!(
                            "thread_do: failed to delete stranded state:{}: {}",
                            ev_id,
                            e
                        );
                    }
                }

                Response::from_json(&checkpoint)
            }
            (Method::Get, "/latest") => {
                let storage = self.state.storage();
                let latest: Option<Checkpoint> = storage.get("latest").await.ok().flatten();

                if let Some(cp) = latest {
                    let state: Option<serde_json::Value> = storage
                        .get(&format!("state:{}", cp.id))
                        .await
                        .ok()
                        .flatten();
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
                let history: VecDeque<Checkpoint> = storage
                    .get("history")
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                Response::from_json(&history)
            }
            _ => Response::error("not found", 404),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_checkpoint(id: &str) -> Checkpoint {
        Checkpoint {
            id: id.to_string(),
            thread_id: "thread-1".to_string(),
            node_id: "n".to_string(),
            parent_id: None,
            state_r2_key: format!("threads/thread-1/{}", id),
            state_size_bytes: Some(0),
            metadata: None,
            created_at: "2026-06-11T00:00:00Z".to_string(),
        }
    }

    // ── PR #132 finding: thread_do.rs:44 — 128KB checkpoint validation ──

    #[test]
    fn validate_checkpoint_size_accepts_small_state() {
        let small = serde_json::json!({ "foo": "bar" });
        let len = serde_json::to_string(&small).unwrap().len();
        assert!(matches!(
            validate_checkpoint_size(len),
            CheckpointSize::Ok(_)
        ));
    }

    #[test]
    fn validate_checkpoint_size_accepts_state_exactly_at_limit() {
        // Boundary: a state whose serialized form is exactly
        // MAX_DO_STATE_BYTES should still be accepted.
        let result = validate_checkpoint_size(MAX_DO_STATE_BYTES);
        assert!(matches!(result, CheckpointSize::Ok(_)));
    }

    #[test]
    fn validate_checkpoint_size_rejects_oversize_state_with_413_carry() {
        // PR #132 crr regression test: a state above the 128 KB limit
        // must be rejected. We assert the observed/limit are carried
        // back so the response message can quote concrete numbers.
        let oversize = MAX_DO_STATE_BYTES + 1;
        let result = validate_checkpoint_size(oversize);
        match result {
            CheckpointSize::TooLarge { observed, limit } => {
                assert_eq!(observed, oversize);
                assert_eq!(limit, 128 * 1024);
            }
            other => panic!("expected TooLarge, got {:?}", other),
        }
    }

    // ── PR #132 finding: thread_do.rs:57 — history-trim cleans state keys ──

    #[test]
    fn ids_to_evict_returns_empty_when_under_limit() {
        let mut history: VecDeque<Checkpoint> = VecDeque::new();
        for i in 0..5 {
            history.push_front(dummy_checkpoint(&format!("c{}", i)));
        }
        let evicted = ids_to_evict_for_trim(&history, MAX_HISTORY_ENTRIES);
        assert!(evicted.is_empty());
    }

    #[test]
    fn ids_to_evict_returns_oldest_first_when_over_limit() {
        // The DO trims at MAX_HISTORY_ENTRIES (50) — when the ring is
        // pushed to 51 the eviction returns the id of the single
        // oldest entry, which the DO uses to delete `state:{id}` and
        // close the leak called out in PR #132 (thread_do.rs:57).
        let mut history: VecDeque<Checkpoint> = VecDeque::new();
        // Insert newest at front; oldest ends up at back. So entry
        // pushed first is `c0`, eventually at the back.
        for i in 0..(MAX_HISTORY_ENTRIES + 1) {
            history.push_front(dummy_checkpoint(&format!("c{}", i)));
        }
        let evicted = ids_to_evict_for_trim(&history, MAX_HISTORY_ENTRIES);
        assert_eq!(evicted, vec!["c0".to_string()]);
    }

    #[test]
    fn ids_to_evict_handles_multiple_overflow() {
        // Defensive: if for some reason multiple entries need to be
        // evicted in one call (e.g. lowered limit), we return all of
        // them, oldest-first, so the DO deletes every stranded
        // state:{id} key on a single tick.
        let mut history: VecDeque<Checkpoint> = VecDeque::new();
        for i in 0..5 {
            history.push_front(dummy_checkpoint(&format!("c{}", i)));
        }
        // history is now [c4, c3, c2, c1, c0] (front to back).
        let evicted = ids_to_evict_for_trim(&history, 2);
        // Should evict c0, c1, c2 (oldest 3).
        assert_eq!(
            evicted,
            vec!["c0".to_string(), "c1".to_string(), "c2".to_string()]
        );
    }

    // ── DO unit coverage (PR #142): history ring + size boundary ──────

    fn make_cp(id: &str) -> Checkpoint {
        dummy_checkpoint(id)
    }

    #[test]
    fn append_checkpoint_then_history_returns_expected_order() {
        let mut history: VecDeque<Checkpoint> = VecDeque::new();
        append_to_history(&mut history, make_cp("a"), MAX_HISTORY_ENTRIES);
        append_to_history(&mut history, make_cp("b"), MAX_HISTORY_ENTRIES);
        append_to_history(&mut history, make_cp("c"), MAX_HISTORY_ENTRIES);

        let ids: Vec<_> = history.iter().map(|cp| cp.id.clone()).collect();
        assert_eq!(ids, vec!["c", "b", "a"]);
    }

    #[test]
    fn history_is_trimmed_at_documented_cap() {
        let mut history: VecDeque<Checkpoint> = VecDeque::new();
        for i in 0..(MAX_HISTORY_ENTRIES + 5) {
            append_to_history(&mut history, make_cp(&format!("cp-{i:03}")), MAX_HISTORY_ENTRIES);
        }

        assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
        let ids: Vec<&str> = history.iter().map(|cp| cp.id.as_str()).collect();
        assert!(!ids.contains(&"cp-000"));
        assert_eq!(
            history.front().unwrap().id,
            format!("cp-{:03}", MAX_HISTORY_ENTRIES + 4)
        );
    }

    #[test]
    fn checkpoint_at_boundary_size_is_accepted() {
        let payload: String = "a".repeat(MAX_DO_STATE_BYTES - 2);
        let state = serde_json::Value::String(payload);
        assert_eq!(measured_state_size(&state), MAX_DO_STATE_BYTES);
        assert!(matches!(
            validate_checkpoint_size(measured_state_size(&state)),
            CheckpointSize::Ok(_)
        ));
    }

    #[test]
    fn checkpoint_over_cap_is_rejected() {
        let payload: String = "b".repeat(MAX_DO_STATE_BYTES);
        let state = serde_json::Value::String(payload);
        let sz = measured_state_size(&state);
        assert!(sz > MAX_DO_STATE_BYTES);
        assert!(matches!(
            validate_checkpoint_size(sz),
            CheckpointSize::TooLarge { .. }
        ));
    }
}
