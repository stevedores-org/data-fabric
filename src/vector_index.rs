use serde_json::json;
use wasm_bindgen::{JsCast, JsValue};
use worker::*;

pub struct SemanticIndex {
    ai: Ai,
    index: JsValue,
}

impl SemanticIndex {
    pub fn new(env: &Env) -> Result<Self> {
        let ai = env.ai("AI")?;
        let index = js_sys::Reflect::get(env, &JsValue::from_str("SEMANTIC_INDEX"))
            .map_err(|e| Error::RustError(format!("failed to get SEMANTIC_INDEX: {:?}", e)))?;
        if index.is_undefined() {
            return Err(Error::RustError("SEMANTIC_INDEX binding is undefined".into()));
        }
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
            "id": id,
            "values": vector,
            "metadata": metadata
        });

        // In JS, insert takes an array of vectors: index.insert([ { id, values, metadata } ])
        let vectors = js_sys::Array::new();
        vectors.push(&serde_wasm_bindgen::to_value(&body)
            .map_err(|e| Error::RustError(e.to_string()))?);

        let insert_fn = js_sys::Reflect::get(&self.index, &JsValue::from_str("insert"))
            .map_err(|e| Error::RustError(format!("failed to get insert method: {:?}", e)))?;
        let insert_fn: js_sys::Function = insert_fn.dyn_into()
            .map_err(|_| Error::RustError("insert is not a function".into()))?;

        let promise = insert_fn.call1(&self.index, &vectors)
            .map_err(|e| Error::RustError(format!("failed to call insert: {:?}", e)))?;
        let promise: js_sys::Promise = promise.dyn_into()
            .map_err(|_| Error::RustError("insert did not return a promise".into()))?;

        wasm_bindgen_futures::JsFuture::from(promise).await
            .map_err(|e| Error::RustError(format!("insert promise failed: {:?}", e)))?;

        Ok(())
    }

    pub async fn query(&self, vector: Vec<f32>, top_k: usize) -> Result<serde_json::Value> {
        // Convert vector to JsValue (which will be a JS array of numbers)
        let js_vector = serde_wasm_bindgen::to_value(&vector)
            .map_err(|e| Error::RustError(e.to_string()))?;

        // Convert options to JsValue
        let options = json!({
            "topK": top_k,
            "returnMetadata": "all"
        });
        let js_options = serde_wasm_bindgen::to_value(&options)
            .map_err(|e| Error::RustError(e.to_string()))?;

        let query_fn = js_sys::Reflect::get(&self.index, &JsValue::from_str("query"))
            .map_err(|e| Error::RustError(format!("failed to get query method: {:?}", e)))?;
        let query_fn: js_sys::Function = query_fn.dyn_into()
            .map_err(|_| Error::RustError("query is not a function".into()))?;

        let promise = query_fn.call2(&self.index, &js_vector, &js_options)
            .map_err(|e| Error::RustError(format!("failed to call query: {:?}", e)))?;
        let promise: js_sys::Promise = promise.dyn_into()
            .map_err(|_| Error::RustError("query did not return a promise".into()))?;

        let result_js = wasm_bindgen_futures::JsFuture::from(promise).await
            .map_err(|e| Error::RustError(format!("query promise failed: {:?}", e)))?;

        let result: serde_json::Value = serde_wasm_bindgen::from_value(result_js)
            .map_err(|e| Error::RustError(e.to_string()))?;

        Ok(result)
    }
}
