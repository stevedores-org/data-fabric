use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DataFabricClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API returned error status {status}: {message}")]
    ApiError { status: StatusCode, message: String },

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, DataFabricClientError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub service: String,
    pub status: String,
    pub mission: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRunRequest {
    pub repo: String,
    pub trigger: Option<String>,
    pub actor: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCreated {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunsListResponse {
    pub runs: Vec<Value>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCheckRequest {
    pub action: String,
    pub actor: String,
    pub resource: Option<String>,
    pub context: Option<Value>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCheckResponse {
    pub id: String,
    pub action: String,
    pub decision: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub id: String,
    pub run_id: Option<String>,
    pub thread_id: Option<String>,
    pub event_type: String,
    pub node_id: Option<String>,
    pub actor: Option<String>,
    pub payload: Option<Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceResponse {
    pub run_id: String,
    pub events: Vec<TraceEvent>,
    pub total: Option<u64>,
    pub truncated: Option<bool>,
}

pub struct DataFabricClient {
    client: Client,
    base_url: String,
    tenant_id: String,
    auth_token: Option<String>,
}

impl DataFabricClient {
    pub fn new(base_url: &str, tenant_id: &str, auth_token: Option<&str>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            tenant_id: tenant_id.to_string(),
            auth_token: auth_token.map(|s| s.to_string()),
        }
    }

    fn apply_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut builder = builder
            .header("x-tenant-id", &self.tenant_id)
            .header("content-type", "application/json");
        if let Some(ref token) = self.auth_token {
            builder = builder.header("authorization", format!("Bearer {}", token));
        }
        builder
    }

    pub async fn health(&self) -> Result<HealthResponse> {
        let url = format!("{}/health", self.base_url);
        let resp = self.client.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn provision_tenant(&self, display_name: &str) -> Result<Value> {
        let url = format!("{}/v1/tenants/provision", self.base_url);
        let body = serde_json::json!({
            "tenant_id": self.tenant_id,
            "display_name": display_name,
        });
        let req = self.client.post(&url).json(&body);
        let resp = self.apply_headers(req).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn create_run(&self, req: CreateRunRequest) -> Result<RunCreated> {
        let url = format!("{}/v1/runs", self.base_url);
        let req_builder = self.client.post(&url).json(&req);
        let resp = self.apply_headers(req_builder).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn list_runs(&self, repo: Option<&str>, limit: Option<usize>, cursor: Option<&str>) -> Result<RunsListResponse> {
        let url = format!("{}/v1/runs", self.base_url);
        let mut query = HashMap::new();
        if let Some(r) = repo { query.insert("repo".to_string(), r.to_string()); }
        if let Some(l) = limit { query.insert("limit".to_string(), l.to_string()); }
        if let Some(c) = cursor { query.insert("cursor".to_string(), c.to_string()); }
        
        let req_builder = self.client.get(&url).query(&query);
        let resp = self.apply_headers(req_builder).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn save_checkpoint(&self, run_id: &str, state: Value, metadata: Option<Value>) -> Result<Value> {
        let url = format!("{}/v1/checkpoints", self.base_url);
        let body = serde_json::json!({
            "run_id": run_id,
            "state": state,
            "metadata": metadata,
        });
        let req_builder = self.client.post(&url).json(&body);
        let resp = self.apply_headers(req_builder).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn get_checkpoint(&self, id: &str) -> Result<Value> {
        let url = format!("{}/v1/checkpoints/{}", self.base_url, id);
        let req_builder = self.client.get(&url);
        let resp = self.apply_headers(req_builder).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn delete_checkpoint(&self, id: &str) -> Result<()> {
        let url = format!("{}/v1/checkpoints/{}", self.base_url, id);
        let req_builder = self.client.delete(&url);
        let resp = self.apply_headers(req_builder).send().await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn upload_artifact(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        let url = format!("{}/v1/artifacts/{}", self.base_url, key);
        let req_builder = self.client.put(&url)
            .body(bytes)
            .header("content-type", "application/octet-stream");
        let resp = self.apply_headers(req_builder).send().await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn download_artifact(&self, key: &str) -> Result<Vec<u8>> {
        let url = format!("{}/v1/artifacts/{}", self.base_url, key);
        let req_builder = self.client.get(&url);
        let resp = self.apply_headers(req_builder).send().await?;
        if resp.status().is_success() {
            Ok(resp.bytes().await?.to_vec())
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn check_policy(&self, req: PolicyCheckRequest) -> Result<PolicyCheckResponse> {
        let url = format!("{}/v1/policies/check", self.base_url);
        let req_builder = self.client.post(&url).json(&req);
        let resp = self.apply_headers(req_builder).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }

    pub async fn get_trace(&self, run_id: &str, limit: Option<usize>) -> Result<TraceResponse> {
        let url = format!("{}/v1/traces/{}", self.base_url, run_id);
        let mut query = HashMap::new();
        if let Some(l) = limit {
            query.insert("limit".to_string(), l.to_string());
        }
        let req_builder = self.client.get(&url).query(&query);
        let resp = self.apply_headers(req_builder).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Err(DataFabricClientError::ApiError {
                status: resp.status(),
                message: resp.text().await?,
            })
        }
    }
}
