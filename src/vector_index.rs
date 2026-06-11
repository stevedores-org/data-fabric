use serde_json::json;
use worker::*;

pub struct SemanticIndex {
    ai: Ai,
    index: Fetcher,
}

impl SemanticIndex {
    pub fn new(env: &Env) -> Result<Self> {
        let ai = env.ai("AI")?;
        let index = env.service("SEMANTIC_INDEX")?;
        Ok(Self { ai, index })
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let result: serde_json::Value = self
            .ai
            .run("@cf/baai/bge-base-en-v1.5", json!({ "text": [text] }))
            .await?;

        // Parse embedding from result
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

        self.index.fetch_request(req).await?;
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

        let mut resp = self.index.fetch_request(req).await?;
        resp.json().await
    }
}
