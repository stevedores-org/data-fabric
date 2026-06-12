pub mod types;

use types::*;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use reqwest::{Client as HttpClient, Method, RequestBuilder, StatusCode};
use serde::{de::DeserializeOwned, Serialize};

/// Reserved character set for a single URI path segment per RFC 3986. Anything
/// outside `pchar` (which excludes `/`, `?`, `#`, etc.) must be percent-encoded
/// or a caller could smuggle additional path segments or query strings into
/// the URL via a crafted ID.
const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');

fn encode_path_segment(s: &str) -> String {
    utf8_percent_encode(s, PATH_SEGMENT_ENCODE_SET).to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("API error (status {status}): {message}")]
    Api {
        status: StatusCode,
        message: String,
    },

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
        
        let tenant_id = std::env::var("DATA_FABRIC_TENANT_ID")
            .map_err(|_| Error::Config("DATA_FABRIC_TENANT_ID environment variable is missing".to_string()))?;
        
        let tenant_role = std::env::var("DATA_FABRIC_TENANT_ROLE")
            .unwrap_or_else(|_| "builder".to_string());

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
        let mut req = self.http.request(method, url)
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

    // ── Runs ───────────────────────────────────────────────────────────────
    
    pub async fn create_run(&self, run: &CreateRun) -> Result<Created> {
        self.send_request(Method::POST, "/v1/runs", Some(run)).await
    }

    pub async fn list_runs(
        &self,
        repo: Option<&str>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<serde_json::Value> {
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
        self.send_request::<(), serde_json::Value>(Method::GET, &path, None).await
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
        self.send_request::<(), serde_json::Value>(Method::GET, &path, None).await
    }

    // ── Agent Tasks ────────────────────────────────────────────────────────
    
    pub async fn create_agent_task(&self, task: &CreateAgentTask) -> Result<TaskCreated> {
        self.send_request(Method::POST, "/v1/tasks", Some(task)).await
    }

    /// Build the HTTP request used by [`Client::claim_next_task`].
    ///
    /// Extracted so the method/URL/query-param shape can be unit-tested without
    /// spinning up a mock server. The new contract (see PR #140) is:
    ///
    /// * `POST /mcp/task/next` — the operation is state-mutating (transfers
    ///   ownership of a task), so it must use POST. GET now returns 405.
    /// * `agent_id` is a required query parameter.
    /// * `cap` is an optional comma-separated query parameter.
    /// * No request body — inputs come exclusively from the query string.
    ///
    /// Query parameters are appended via `reqwest::RequestBuilder::query`, which
    /// percent-encodes each value. Interpolating user-controlled inputs into the
    /// URL string with `format!` would let a caller smuggle extra parameters
    /// (e.g. `agent_id="agent-1&cap=evil"` would inject a second `cap`).
    pub fn build_claim_next_task_request(
        &self,
        agent_id: &str,
        capabilities: &[String],
    ) -> Result<reqwest::Request> {
        let mut builder = self
            .prepare_request(Method::POST, "/mcp/task/next")
            .query(&[("agent_id", agent_id)]);
        if !capabilities.is_empty() {
            let caps = capabilities.join(",");
            builder = builder.query(&[("cap", caps.as_str())]);
        }
        builder.build().map_err(Error::from)
    }

    pub async fn claim_next_task(&self, agent_id: &str, capabilities: &[String]) -> Result<Option<AgentTask>> {
        let req = self.build_claim_next_task_request(agent_id, capabilities)?;
        let resp = self.http.execute(req).await?;
        if resp.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let task = self.handle_response::<AgentTask>(resp).await?;
        Ok(Some(task))
    }

    pub async fn heartbeat_task(&self, task_id: &str, agent_id: &str) -> Result<serde_json::Value> {
        let req = self.build_heartbeat_task_request(task_id, agent_id)?;
        let resp = self.http.execute(req).await?;
        self.handle_response(resp).await
    }

    /// Build the HTTP request used by [`Client::heartbeat_task`].
    ///
    /// `task_id` is percent-encoded into the path segment and `agent_id` is
    /// appended as a percent-encoded query parameter (`reqwest`'s `.query()`
    /// builder handles encoding). Interpolating either value with `format!`
    /// would let a caller inject extra path segments or query parameters via
    /// a crafted ID like `task_id = "abc/extra?injected=1"`.
    pub fn build_heartbeat_task_request(
        &self,
        task_id: &str,
        agent_id: &str,
    ) -> Result<reqwest::Request> {
        let path = format!("/mcp/task/{}/heartbeat", encode_path_segment(task_id));
        self.prepare_request(Method::POST, &path)
            .query(&[("agent_id", agent_id)])
            .build()
            .map_err(Error::from)
    }

    pub async fn complete_task(&self, task_id: &str, req: &TaskCompleteRequest) -> Result<serde_json::Value> {
        let path = format!("/mcp/task/{}/complete", task_id);
        self.send_request(Method::POST, &path, Some(req)).await
    }

    pub async fn fail_task(&self, task_id: &str, req: &TaskFailRequest) -> Result<serde_json::Value> {
        let path = format!("/mcp/task/{}/fail", task_id);
        self.send_request(Method::POST, &path, Some(req)).await
    }

    // ── Agents ─────────────────────────────────────────────────────────────
    
    pub async fn register_agent(&self, agent: &RegisterAgent) -> Result<serde_json::Value> {
        self.send_request(Method::POST, "/v1/agents", Some(agent)).await
    }

    pub async fn list_agents(&self) -> Result<serde_json::Value> {
        self.send_request::<(), serde_json::Value>(Method::GET, "/v1/agents", None).await
    }

    // ── Checkpoints ────────────────────────────────────────────────────────
    
    pub async fn create_checkpoint(&self, checkpoint: &CreateCheckpoint) -> Result<CheckpointCreated> {
        self.send_request(Method::POST, "/v1/checkpoints", Some(checkpoint)).await
    }

    pub async fn get_latest_checkpoint_for_thread(&self, thread_id: &str) -> Result<serde_json::Value> {
        let path = format!("/v1/checkpoints/threads/{}", thread_id);
        self.send_request::<(), serde_json::Value>(Method::GET, &path, None).await
    }

    pub async fn get_checkpoint(&self, id: &str) -> Result<Checkpoint> {
        let path = format!("/v1/checkpoints/{}", id);
        self.send_request::<(), Checkpoint>(Method::GET, &path, None).await
    }

    pub async fn delete_checkpoint(&self, id: &str) -> Result<serde_json::Value> {
        let path = format!("/v1/checkpoints/{}", id);
        self.send_request::<(), serde_json::Value>(Method::DELETE, &path, None).await
    }

    // ── Artifacts ──────────────────────────────────────────────────────────
    
    pub async fn put_artifact(&self, key: &str, data: Vec<u8>) -> Result<ArtifactStoredResponse> {
        let path = format!("/v1/artifacts/{}", key);
        let resp = self.prepare_request(Method::PUT, &path)
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
    
    pub async fn check_policy(&self, req: &PolicyCheckRequest) -> Result<PolicyCheckResponse> {
        self.send_request(Method::POST, "/v1/policies/check", Some(req)).await
    }

    // ── Metrics ────────────────────────────────────────────────────────────
    
    pub async fn get_pilot_metrics(&self, window: Option<&str>, task_type: Option<&str>) -> Result<PilotMetrics> {
        let mut path = "/v1/metrics/pilot?".to_string();
        if let Some(w) = window {
            path.push_str(&format!("window={}&", w));
        }
        if let Some(t) = task_type {
            path.push_str(&format!("task_type={}&", t));
        }
        self.send_request::<(), PilotMetrics>(Method::GET, &path, None).await
    }

    // ── Graph Events ───────────────────────────────────────────────────────
    
    pub async fn post_graph_events(&self, batch: &GraphEventBatch) -> Result<GraphEventAck> {
        self.send_request(Method::POST, "/v1/graph-events", Some(batch)).await
    }

    // ── Integrations (WS6) ─────────────────────────────────────────────────
    
    pub async fn ingest_oxidizedgraph_events(&self, batch: &GraphExecBatch) -> Result<serde_json::Value> {
        self.send_request(Method::POST, "/v1/integrations/oxidizedgraph/events", Some(batch)).await
    }

    pub async fn ingest_aivcs_events(&self, evt: &PipelineEvent) -> Result<serde_json::Value> {
        self.send_request(Method::POST, "/v1/integrations/aivcs/events", Some(evt)).await
    }

    pub async fn submit_llama_inference(&self, req: &InferenceRequest) -> Result<serde_json::Value> {
        self.send_request(Method::POST, "/v1/integrations/llama-rs/inference", Some(req)).await
    }

    pub async fn ingest_llama_telemetry(&self, telemetry: &InferenceTelemetry) -> Result<serde_json::Value> {
        self.send_request(Method::POST, "/v1/integrations/llama-rs/telemetry", Some(telemetry)).await
    }

    pub async fn get_llama_context(&self, req: &ContextRequest) -> Result<ContextPackResponse> {
        self.send_request(Method::POST, "/v1/integrations/llama-rs/context", Some(req)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> Client {
        Client::new(ClientConfig {
            base_url: "https://df.example".to_string(),
            tenant_id: "t-1".to_string(),
            tenant_role: "builder".to_string(),
            cf_client_id: None,
            cf_client_secret: None,
        })
    }

    /// Regression test for PR #140: `/mcp/task/next` flipped from GET to POST
    /// in the worker, and the GET shim now returns 405. Every call from this
    /// client must use POST or it will fail at runtime.
    #[test]
    fn claim_next_task_uses_post() {
        let client = test_client();
        let req = client
            .build_claim_next_task_request("agent-1", &["rust".to_string(), "wasm".to_string()])
            .expect("request must build");
        assert_eq!(req.method(), &Method::POST, "must use POST per PR #140 contract");
    }

    /// `agent_id` and `cap` belong on the query string (the new POST handler
    /// reads them from `url.query_pairs()`), not in a JSON body. We assert
    /// against the *decoded* query pairs because the URL builder may percent-
    /// encode sub-delims like `,` — that's still a valid `cap=rust,wasm` from
    /// the server's perspective (which decodes via `url.query_pairs()`).
    #[test]
    fn claim_next_task_puts_inputs_in_query_string_not_body() {
        let client = test_client();
        let req = client
            .build_claim_next_task_request("agent-1", &["rust".to_string(), "wasm".to_string()])
            .expect("request must build");
        let url = req.url();
        assert_eq!(url.path(), "/mcp/task/next");
        let pairs: std::collections::HashMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(pairs.get("agent_id").map(String::as_str), Some("agent-1"));
        assert_eq!(pairs.get("cap").map(String::as_str), Some("rust,wasm"));
        assert!(req.body().is_none(), "POST body must be empty; the handler reads from query params");
    }

    /// `cap` is optional per the OpenAPI spec — omit it entirely when no
    /// capabilities are supplied (rather than sending `cap=`), so the
    /// server-side parser doesn't see an empty capability set as a single
    /// empty-string capability.
    #[test]
    fn claim_next_task_omits_cap_when_no_capabilities() {
        let client = test_client();
        let req = client
            .build_claim_next_task_request("agent-1", &[])
            .expect("request must build");
        let query = req.url().query().unwrap_or("");
        assert!(query.contains("agent_id=agent-1"));
        assert!(!query.contains("cap="), "cap must be omitted when no capabilities are supplied, got: {query}");
    }

    /// Regression test: `agent_id` and the `cap` values must be percent-encoded
    /// so a malicious caller can't smuggle an extra query parameter through the
    /// URL. Before the fix, `agent_id="user@host&cap=evil"` was interpolated
    /// raw and injected a second `cap=evil` parameter that the server would
    /// then merge with the legitimate `cap=rust`.
    #[test]
    fn claim_next_task_encodes_special_chars_in_agent_id() {
        let client = test_client();
        let req = client
            .build_claim_next_task_request("user@host&cap=evil", &["rust".to_string()])
            .expect("request must build");
        let url = req.url();
        let query = url.query().expect("query string must be present");

        // The `&` and `=` in the supplied agent_id must be percent-encoded so
        // they appear inside the single agent_id value, not as separators.
        assert!(
            !query.contains("agent_id=user@host&cap=evil"),
            "agent_id must be percent-encoded; raw injection allowed extra cap, got: {query}"
        );

        // Exactly one `cap` parameter must be present (the legitimate one).
        let cap_count = url
            .query_pairs()
            .filter(|(k, _)| k == "cap")
            .count();
        assert_eq!(cap_count, 1, "expected exactly one cap param, got {cap_count} in {query}");

        // The agent_id pair must round-trip to the original (decoded) value.
        let agent_id_value = url
            .query_pairs()
            .find(|(k, _)| k == "agent_id")
            .map(|(_, v)| v.into_owned())
            .expect("agent_id must be present");
        assert_eq!(agent_id_value, "user@host&cap=evil");
    }

    /// Regression test: `task_id` is a path segment and `agent_id` is a query
    /// parameter — both must be percent-encoded. Before the fix,
    /// `task_id="abc/extra?injected=1"` was interpolated raw into the path and
    /// turned a heartbeat into a request against `/mcp/task/abc/extra/heartbeat`
    /// with a synthetic `injected=1` query parameter.
    #[test]
    fn heartbeat_task_encodes_special_chars_in_path_and_query() {
        let client = test_client();
        let req = client
            .build_heartbeat_task_request("abc/extra?injected=1", "agent&id=evil")
            .expect("request must build");
        let url = req.url();
        assert_eq!(req.method(), &Method::POST);

        // The crafted `/` and `?` in task_id must be percent-encoded inside
        // a single path segment, not interpreted as path/query separators.
        let path = url.path();
        assert!(
            path.starts_with("/mcp/task/") && path.ends_with("/heartbeat"),
            "path must wrap an encoded task_id segment, got: {path}"
        );
        assert!(
            !path.contains("/extra/"),
            "task_id slash must be encoded, got: {path}"
        );
        assert!(
            !path.contains('?'),
            "task_id question mark must be encoded, got: {path}"
        );

        // Exactly one `agent_id` query parameter; no injected ones.
        let agent_count = url.query_pairs().filter(|(k, _)| k == "agent_id").count();
        assert_eq!(agent_count, 1, "expected exactly one agent_id param");

        // No injected `injected=1` parameter from the task_id.
        assert!(
            url.query_pairs().all(|(k, _)| k != "injected"),
            "task_id must not be able to inject query params: {:?}",
            url.query()
        );

        // agent_id round-trips with its `&` and `=` preserved as data.
        let agent_id_value = url
            .query_pairs()
            .find(|(k, _)| k == "agent_id")
            .map(|(_, v)| v.into_owned())
            .expect("agent_id must be present");
        assert_eq!(agent_id_value, "agent&id=evil");
    }
}
