//! Semantic indexing helper backed by Workers AI + Vectorize.
//!
//! KNOWN LIMITATION (tracked in follow-up issue): the `SEMANTIC_INDEX` binding
//! is declared as `[[vectorize]]` in `wrangler.toml`, but worker-rs 0.8.x has
//! no typed `Env::vectorize()` accessor — only `ai`, `analytics_engine`,
//! `service`, `kv`, `d1`, etc. (see `Env` in worker-0.8.3/src/env.rs).
//!
//! We currently fall back to `env.service("SEMANTIC_INDEX")` which returns a
//! `Fetcher`. That works at compile time but the live binding object is a
//! Vectorize index, not a Fetcher — so the `fetch_request` calls in `insert`
//! and `query` fail at runtime. Every caller in `lib.rs` wraps these in
//! `if let Ok(...)` so the failure is silent — semantic indexing is a no-op
//! today and the worker continues without it.
//!
//! Three options to actually fix this:
//!   1. Upgrade `worker` past the version that adds typed Vectorize support
//!      and replace this whole module with the typed binding.
//!   2. Hand-roll a Vectorize REST client using `reqwest`/`fetch` against
//!      `api.cloudflare.com` (requires an API-token secret binding, not the
//!      Vectorize binding).
//!   3. Stand up a small JS Worker that exposes a fetch handler wrapping
//!      `env.SEMANTIC_INDEX.{insert,query}`, and bind to it as a service
//!      binding from this Rust worker (so `env.service(...)` is correct).
//!
//! Until then the `fail_*` helpers at the bottom of this module log a
//! diagnostic so the broken path is observable (`wrangler tail`) instead of
//! silently dropping memory items.

use serde_json::json;
use worker::*;

pub struct SemanticIndex {
    ai: Ai,
    index: Fetcher,
}

impl SemanticIndex {
    pub fn new(env: &Env) -> Result<Self> {
        let ai = env.ai("AI").inspect_err(|e| {
            console_warn!(
                "vector_index: env.ai(\"AI\") failed — semantic indexing disabled ({e:?})"
            );
        })?;
        // KNOWN BUG: see module doc comment. `SEMANTIC_INDEX` is a Vectorize
        // binding, not a service binding, so the returned Fetcher's
        // fetch_request will fail at runtime against a real CF deployment.
        let index = env.service("SEMANTIC_INDEX").inspect_err(|e| {
            console_warn!(
                "vector_index: env.service(\"SEMANTIC_INDEX\") failed — semantic indexing disabled ({e:?})"
            );
        })?;
        Ok(Self { ai, index })
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let result: serde_json::Value = self
            .ai
            .run("@cf/baai/bge-base-en-v1.5", json!({ "text": [text] }))
            .await
            .inspect_err(|e| {
                console_warn!("vector_index: AI.run failed — returning embed error ({e:?})");
            })?;

        // Workers AI result format for embeddings: {"data": [[0.1, ...]], "shape": [1, 768]}
        let embedding = result["data"][0]
            .as_array()
            .ok_or_else(|| Error::RustError("failed to parse embedding".into()))?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        Ok(embedding)
    }

    pub async fn insert(
        &self,
        id: &str,
        vector: Vec<f32>,
        metadata: serde_json::Value,
    ) -> Result<()> {
        let body = json!({
            "vectors": [{
                "id": id,
                "values": vector,
                "metadata": metadata
            }]
        });

        let req = Request::new_with_init(
            "http://vectorize/insert",
            &RequestInit {
                method: Method::Post,
                body: Some(
                    serde_wasm_bindgen::to_value(&body)
                        .map_err(|e| Error::RustError(e.to_string()))?,
                ),
                ..Default::default()
            },
        )?;

        // See module doc comment — this call is expected to fail at runtime
        // against the live Vectorize binding. The caller handles the error.
        self.index.fetch_request(req).await.inspect_err(|e| {
            console_warn!(
                "vector_index: SEMANTIC_INDEX.fetch_request(insert) failed (id={id}) — binding-type mismatch, see vector_index.rs doc comment ({e:?})"
            );
        })?;
        Ok(())
    }

    pub async fn query(&self, vector: Vec<f32>, top_k: usize) -> Result<serde_json::Value> {
        let body = json!({
            "vector": vector,
            "topK": top_k,
            "returnMetadata": "all"
        });

        let req = Request::new_with_init(
            "http://vectorize/query",
            &RequestInit {
                method: Method::Post,
                body: Some(
                    serde_wasm_bindgen::to_value(&body)
                        .map_err(|e| Error::RustError(e.to_string()))?,
                ),
                ..Default::default()
            },
        )?;

        // See module doc comment — expected to fail at runtime.
        let mut resp = self.index.fetch_request(req).await.inspect_err(|e| {
            console_warn!(
                "vector_index: SEMANTIC_INDEX.fetch_request(query) failed (top_k={top_k}) — binding-type mismatch, see vector_index.rs doc comment ({e:?})"
            );
        })?;
        resp.json().await
    }
}
