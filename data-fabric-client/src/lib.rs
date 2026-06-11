pub mod types;

pub use types::*;
pub type DataFabricClient = Client;
pub type CreateRunRequest = CreateRun;

use reqwest::{Client as HttpClient, Method, RequestBuilder, StatusCode};
use serde::{de::DeserializeOwned, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("API error (status {status}): {message}")]
    Api { status: StatusCode, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub base_url: String,
    pub tenant_id: String,
    pub tenant_role: String,
    pub cf_client_id: Option<String>,
    pub cf_client_secret: Option<String>,
}

fn read_secret_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl ClientConfig {
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("DATA_FABRIC_URL")
            .unwrap_or_else(|_| "https://data-fabric.data-fabric.svc.cluster.local".to_string());

        let tenant_id = std::env::var("DATA_FABRIC_TENANT_ID").map_err(|_| {
            Error::Config("DATA_FABRIC_TENANT_ID environment variable is missing".to_string())
        })?;

        let tenant_role =
            std::env::var("DATA_FABRIC_TENANT_ROLE").unwrap_or_else(|_| "builder".to_string());

        // Read from files first
        let mut cf_client_id = read_secret_file("/var/run/secrets/data-fabric/client-id");
        let mut cf_client_secret = read_secret_file("/var/run/secrets/data-fabric/client-secret");

        // Fallback to env vars
        if cf_client_id.is_none() {
            cf_client_id = std::env::var("CF_ACCESS_CLIENT_ID").ok();
        }
        if cf_client_secret.is_none() {
            cf_client_secret = std::env::var("CF_ACCESS_CLIENT_SECRET").ok();
        }

        Ok(Self {
            base_url,
            tenant_id,
            tenant_role,
            cf_client_id,
            cf_client_secret,
        })
    }
}

pub struct Client {
    config: ClientConfig,
    http: HttpClient,
}

impl Client {
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config,
            http: HttpClient::new(),
        }
    }

    pub fn from_env() -> Result<Self> {
        let config = ClientConfig::from_env()?;
        Ok(Self::new(config))
    }

    fn prepare_request(&self, method: Method, path: &str) -> RequestBuilder {
        let url = format!("{}{}", self.config.base_url.trim_end_matches('/'), path);
        let mut req = self
            .http
            .request(method, url)
            .header("x-tenant-id", &self.config.tenant_id)
            .header("x-tenant-role", &self.config.tenant_role);

        if let Some(ref client_id) = self.config.cf_client_id {
            req = req.header("CF-Access-Client-Id", client_id);
        }
        if let Some(ref client_secret) = self.config.cf_client_secret {
            req = req.header("CF-Access-Client-Secret", client_secret);
        }

        req
    }

    async fn handle_response<R: DeserializeOwned>(&self, response: reqwest::Response) -> Result<R> {
        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(Error::Api { status, message });
        }
        let data = response.json::<R>().await?;
        Ok(data)
    }

    async fn send_request<T: Serialize, R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
    ) -> Result<R> {
        let mut req = self.prepare_request(method, path);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await?;
        self.handle_response(resp).await
    }

    // ── Health ─────────────────────────────────────────────────────────────

    pub async fn check_root(&self) -> Result<String> {
        let resp = self.prepare_request(Method::GET, "/").send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(Error::Api {
                status,
                message: resp.text().await.unwrap_or_default(),
            });
        }
        Ok(resp.text().await?)
    }

    pub async fn health(&self) -> Result<HealthResponse> {
        let resp = self.prepare_request(Method::GET, "/health").send().await?;
        self.handle_response(resp).await
    }

    pub async fn create_run(&self, run: CreateRun) -> Result<Created> {
        self.send_request(Method::POST, "/v1/runs", Some(&run))
            .await
    }

    pub async fn list_runs(
        &self,
        repo: Option<&str>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<RunsResponse> {
        let mut path = "/v1/runs?".to_string();
        if let Some(r) = repo {
            path.push_str(&format!("repo={}&", r));
        }
        if let Some(l) = limit {
            path.push_str(&format!("limit={}&", l));
        }
        if let Some(c) = cursor {
            path.push_str(&format!("cursor={}&", c));
        }
        self.send_request::<(), RunsResponse>(Method::GET, &path, None)
            .await
    }

    pub async fn get_run(&self, id: &str) -> Result<Run> {
        let path = format!("/v1/runs/{}", id);
        self.send_request::<(), Run>(Method::GET, &path, None).await
    }

    // ── WS2 Tasks ──────────────────────────────────────────────────────────

    pub async fn create_task(&self, run_id: &str, task: &CreateTask) -> Result<Created> {
        let path = format!("/v1/runs/{}/tasks", run_id);
        self.send_request(Method::POST, &path, Some(task)).await
    }

    pub async fn list_tasks(&self, run_id: &str) -> Result<serde_json::Value> {
        let path = format!("/v1/runs/{}/tasks", run_id);
        self.send_request::<(), serde_json::Value>(Method::GET, &path, None)
            .await
    }

    // ── Agent Tasks ────────────────────────────────────────────────────────

    pub async fn create_agent_task(&self, task: &CreateAgentTask) -> Result<TaskCreated> {
        self.send_request(Method::POST, "/v1/tasks", Some(task))
            .await
    }

    pub async fn claim_next_task(
        &self,
        agent_id: &str,
        capabilities: &[String],
    ) -> Result<Option<AgentTask>> {
        let caps = capabilities.join(",");
        let path = format!("/mcp/task/next?agent_id={}&cap={}", agent_id, caps);
        let resp = self.prepare_request(Method::GET, &path).send().await?;
        if resp.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let task = self.handle_response::<AgentTask>(resp).await?;
        Ok(Some(task))
    }

    pub async fn heartbeat_task(&self, task_id: &str, agent_id: &str) -> Result<serde_json::Value> {
        let path = format!("/mcp/task/{}/heartbeat?agent_id={}", task_id, agent_id);
        self.send_request::<(), serde_json::Value>(Method::POST, &path, None)
            .await
    }

    pub async fn complete_task(
        &self,
        task_id: &str,
        req: &TaskCompleteRequest,
    ) -> Result<serde_json::Value> {
        let path = format!("/mcp/task/{}/complete", task_id);
        self.send_request(Method::POST, &path, Some(req)).await
    }

    pub async fn fail_task(
        &self,
        task_id: &str,
        req: &TaskFailRequest,
    ) -> Result<serde_json::Value> {
        let path = format!("/mcp/task/{}/fail", task_id);
        self.send_request(Method::POST, &path, Some(req)).await
    }

    // ── Agents ─────────────────────────────────────────────────────────────

    pub async fn register_agent(&self, agent: &RegisterAgent) -> Result<serde_json::Value> {
        self.send_request(Method::POST, "/v1/agents", Some(agent))
            .await
    }

    pub async fn list_agents(&self) -> Result<serde_json::Value> {
        self.send_request::<(), serde_json::Value>(Method::GET, "/v1/agents", None)
            .await
    }

    // ── Checkpoints ────────────────────────────────────────────────────────

    pub async fn create_checkpoint(
        &self,
        checkpoint: &CreateCheckpoint,
    ) -> Result<CheckpointCreated> {
        self.send_request(Method::POST, "/v1/checkpoints", Some(checkpoint))
            .await
    }

    pub async fn get_latest_checkpoint_for_thread(
        &self,
        thread_id: &str,
    ) -> Result<serde_json::Value> {
        let path = format!("/v1/checkpoints/threads/{}", thread_id);
        self.send_request::<(), serde_json::Value>(Method::GET, &path, None)
            .await
    }

    pub async fn get_checkpoint(&self, id: &str) -> Result<Checkpoint> {
        let path = format!("/v1/checkpoints/{}", id);
        self.send_request::<(), Checkpoint>(Method::GET, &path, None)
            .await
    }

    pub async fn delete_checkpoint(&self, id: &str) -> Result<serde_json::Value> {
        let path = format!("/v1/checkpoints/{}", id);
        self.send_request::<(), serde_json::Value>(Method::DELETE, &path, None)
            .await
    }

    // ── Artifacts ──────────────────────────────────────────────────────────

    pub async fn put_artifact(&self, key: &str, data: Vec<u8>) -> Result<ArtifactStoredResponse> {
        let path = format!("/v1/artifacts/{}", key);
        let resp = self
            .prepare_request(Method::PUT, &path)
            .header("content-type", "application/octet-stream")
            .body(data)
            .send()
            .await?;
        self.handle_response(resp).await
    }

    pub async fn get_artifact(&self, key: &str) -> Result<Vec<u8>> {
        let path = format!("/v1/artifacts/{}", key);
        let resp = self.prepare_request(Method::GET, &path).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let message = resp.text().await.unwrap_or_default();
            return Err(Error::Api { status, message });
        }
        let bytes = resp.bytes().await?;
        Ok(bytes.to_vec())
    }

    // ── Policy ─────────────────────────────────────────────────────────────

    pub async fn check_policy(&self, req: PolicyCheckRequest) -> Result<PolicyCheckResponse> {
        self.send_request(Method::POST, "/v1/policies/check", Some(&req))
            .await
    }

    // ── Checkpoint Helpers ──────────────────────────────────────────────────

    pub async fn save_checkpoint(
        &self,
        thread_id: &str,
        state: serde_json::Value,
        metadata: Option<serde_json::Value>,
    ) -> Result<CheckpointCreated> {
        let checkpoint = CreateCheckpoint {
            thread_id: thread_id.to_string(),
            node_id: "manual".to_string(),
            parent_id: None,
            state,
            metadata,
        };
        self.create_checkpoint(&checkpoint).await
    }

    // ── Traces / Provenance ────────────────────────────────────────────────

    pub async fn get_trace(&self, run_id: &str, limit: Option<usize>) -> Result<TraceResponse> {
        let mut path = format!("/v1/traces/{}", run_id);
        if let Some(l) = limit {
            path.push_str(&format!("?limit={}", l));
        }
        self.send_request::<(), TraceResponse>(Method::GET, &path, None)
            .await
    }

    // ── Metrics ────────────────────────────────────────────────────────────

    pub async fn get_pilot_metrics(
        &self,
        window: Option<&str>,
        task_type: Option<&str>,
    ) -> Result<PilotMetrics> {
        let mut path = "/v1/metrics/pilot?".to_string();
        if let Some(w) = window {
            path.push_str(&format!("window={}&", w));
        }
        if let Some(t) = task_type {
            path.push_str(&format!("task_type={}&", t));
        }
        self.send_request::<(), PilotMetrics>(Method::GET, &path, None)
            .await
    }

    // ── Graph Events ───────────────────────────────────────────────────────

    pub async fn post_graph_events(&self, batch: &GraphEventBatch) -> Result<GraphEventAck> {
        self.send_request(Method::POST, "/v1/graph-events", Some(batch))
            .await
    }

    // ── Integrations (WS6) ─────────────────────────────────────────────────

    pub async fn ingest_oxidizedgraph_events(
        &self,
        batch: &GraphExecBatch,
    ) -> Result<serde_json::Value> {
        self.send_request(
            Method::POST,
            "/v1/integrations/oxidizedgraph/events",
            Some(batch),
        )
        .await
    }

    pub async fn ingest_aivcs_events(&self, evt: &PipelineEvent) -> Result<serde_json::Value> {
        self.send_request(Method::POST, "/v1/integrations/aivcs/events", Some(evt))
            .await
    }

    pub async fn submit_llama_inference(
        &self,
        req: &InferenceRequest,
    ) -> Result<serde_json::Value> {
        self.send_request(
            Method::POST,
            "/v1/integrations/llama-rs/inference",
            Some(req),
        )
        .await
    }

    pub async fn ingest_llama_telemetry(
        &self,
        telemetry: &InferenceTelemetry,
    ) -> Result<serde_json::Value> {
        self.send_request(
            Method::POST,
            "/v1/integrations/llama-rs/telemetry",
            Some(telemetry),
        )
        .await
    }

    pub async fn get_llama_context(&self, req: &ContextRequest) -> Result<ContextPackResponse> {
        self.send_request(Method::POST, "/v1/integrations/llama-rs/context", Some(req))
            .await
    }
}
