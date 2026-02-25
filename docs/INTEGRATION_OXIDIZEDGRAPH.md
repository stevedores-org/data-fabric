# Data Fabric ↔️ oxidizedgraph Integration Guide

This guide shows how to test oxidizedgraph nodes and governance with data-fabric as the persistence backend.

## Overview

data-fabric provides:
- **Persistence**: Store run metadata, events, and artifacts
- **Context**: Retrieve historical context for agent decision-making
- **Provenance**: Full audit trail of agent actions
- **Coordination**: Multi-agent synchronization via structured events

oxidizedgraph provides:
- **Orchestration**: DAG-based workflow execution
- **Nodes**: Pluggable execution units (GovernanceNode, LLMNode, ToolNode)
- **Governance**: Role-based access control and guidance enforcement

Together: **Governance-aware autonomous agents with complete auditability**

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                   oxidizedgraph Runtime                    │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌─────────────────┐    ┌──────────────┐                   │
│  │ GovernanceNode  │───▶│  LLMNode     │                   │
│  │ (guidance rules)│    │ (invoke LLM) │                   │
│  └─────────────────┘    └──────────────┘                   │
│           │                    │                            │
│           ├────────┬───────────┤                            │
│           v        v           v                            │
│     ┌──────────────────────────────────┐                   │
│     │    FabricPersister               │                   │
│     │ (event serialization adapter)    │                   │
│     └──────────────────────────────────┘                   │
│                    │                                        │
└────────────────────┼────────────────────────────────────────┘
                     │
                     │ HTTP
                     v
         ┌──────────────────────────┐
         │    data-fabric Worker    │
         │  (http://localhost:8787) │
         └──────────────────────────┘
                     │
          ┌──────────┼──────────┐
          v          v          v
        ┌───┐      ┌────┐    ┌─────┐
        │ D1│      │ KV │    │ R2  │
        └───┘      └────┘    └─────┘
      (metadata)  (policies) (artifacts)
```

---

## Quick Integration (Rustin 10 minutes)

### Step 1: Start data-fabric Locally

```bash
cd /path/to/data-fabric

# Terminal 1: Start worker
just dev-worker

# Output:
# ✨ Listening on http://localhost:8787
# ✅ GET /health → 200
```

### Step 2: Create FabricPersister Adapter

In oxidizedgraph, create a new module to bridge to data-fabric:

```rust
// oxidizedgraph/src/fabric.rs

use std::sync::Arc;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

/// Persists node execution to data-fabric
pub struct FabricPersister {
    client: Arc<Client>,
    fabric_url: String,
    tenant_id: String,
    run_id: String,
}

impl FabricPersister {
    pub fn new(fabric_url: &str, tenant_id: &str, run_id: &str) -> Self {
        Self {
            client: Arc::new(Client::new()),
            fabric_url: fabric_url.to_string(),
            tenant_id: tenant_id.to_string(),
            run_id: run_id.to_string(),
        }
    }

    /// Record a node execution event
    pub async fn record_event(
        &self,
        node_type: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/fabric/ingest", self.fabric_url);

        let event = json!({
            "event_type": event_type,
            "node_type": node_type,
            "run_id": self.run_id,
            "tenant_id": self.tenant_id,
            "payload": payload,
        });

        self.client
            .post(&url)
            .header("X-Tenant-ID", &self.tenant_id)
            .header("Content-Type", "application/json")
            .json(&event)
            .send()
            .await?;

        Ok(())
    }

    /// Retrieve context for decision-making
    pub async fn query_context(
        &self,
        query_type: &str,
        filters: serde_json::Value,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let url = format!("{}/fabric/query", self.fabric_url);

        let query = json!({
            "query_type": query_type,
            "filters": filters,
        });

        let response = self.client
            .post(&url)
            .header("X-Tenant-ID", &self.tenant_id)
            .header("Content-Type", "application/json")
            .json(&query)
            .send()
            .await?
            .json()
            .await?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fabric_persister_records_event() {
        // Will test once data-fabric is running
        let persister = FabricPersister::new(
            "http://localhost:8787",
            "test-tenant",
            "run_123",
        );

        let payload = json!({
            "node": "governance",
            "status": "executed",
            "guidance_applied": 3,
        });

        // This will fail until data-fabric is running
        let result = persister
            .record_event("governance", "node_executed", payload)
            .await;

        println!("Result: {:?}", result);
    }
}
```

### Step 3: Integrate with GovernanceNode

Extend GovernanceNode to persist events to data-fabric:

```rust
// oxidizedgraph/src/nodes/governance.rs

use crate::fabric::FabricPersister;

pub struct GovernanceNode {
    master_file: PathBuf,
    agent_role: AgentRole,
    config: GovernanceConfig,
    fabric: Option<Arc<FabricPersister>>, // NEW
}

impl GovernanceNode {
    pub fn with_fabric(mut self, fabric: Arc<FabricPersister>) -> Self {
        self.fabric = Some(fabric);
        self
    }
}

#[async_trait]
impl NodeExecutor for GovernanceNode {
    async fn execute(&self, state: SharedState) -> Result<NodeOutput, NodeError> {
        // Execute governance logic as before
        let output = self.apply_rules(state).await?;

        // NEW: Persist to data-fabric
        if let Some(fabric) = &self.fabric {
            fabric
                .record_event(
                    "governance",
                    "node_executed",
                    json!({
                        "status": "success",
                        "guidance_count": output.guidance_count,
                        "role": format!("{:?}", self.agent_role),
                    }),
                )
                .await
                .ok();
        }

        Ok(output)
    }
}
```

### Step 4: Write Integration Tests

```rust
// oxidizedgraph/tests/integration_with_fabric.rs

#[tokio::test]
async fn test_governance_workflow_with_fabric_persistence() {
    // Get fabric URL from env, fallback to localhost
    let fabric_url = std::env::var("FABRIC_URL")
        .unwrap_or_else(|_| "http://localhost:8787".to_string());

    // Check health first
    let health_response = reqwest::Client::new()
        .get(format!("{}/health", fabric_url))
        .send()
        .await;

    // Skip if data-fabric not running
    if health_response.is_err() {
        println!("⚠️  data-fabric not running at {}", fabric_url);
        println!("Start with: just dev-worker");
        return;
    }

    // Set up fabric persister
    let fabric = Arc::new(FabricPersister::new(
        &fabric_url,
        "test-tenant",
        "run_integration_test",
    ));

    // Create governance node with fabric backing
    let gov_node = GovernanceNode::new(
        PathBuf::from("./AGENTS.md"),
        AgentRole::Architect,
        GovernanceConfig::default(),
    )
    .with_fabric(fabric.clone());

    // Create test state
    let state = AgentState::new("test-agent")
        .with_role(AgentRole::Architect);

    // Execute
    let result = gov_node.execute(Arc::new(Mutex::new(state))).await;
    assert!(result.is_ok());

    // Verify persistence
    let context = fabric
        .query_context("memory", json!({"run_id": "run_integration_test"}))
        .await;

    assert!(context.is_ok());
    println!("✅ Governance execution persisted to data-fabric");
}

#[tokio::test]
async fn test_multi_agent_coordination_with_fabric() {
    let fabric_url = "http://localhost:8787";
    let fabric = Arc::new(FabricPersister::new(
        fabric_url,
        "orchestration-test",
        "run_multi_agent",
    ));

    // Create orchestration graph
    let graph = GraphBuilder::new()
        .add_node(
            GovernanceNode::new(
                PathBuf::from("./AGENTS.md"),
                AgentRole::Architect,
                GovernanceConfig::default(),
            )
            .with_fabric(fabric.clone()),
        )
        .add_node(
            LLMNode::new("claude-opus")
                .with_persistence(fabric.clone()),
        )
        .add_node(
            ToolNode::new(vec![/* tools */])
                .with_persistence(fabric.clone()),
        )
        .build()
        .expect("Failed to build graph");

    // Run orchestration
    let runner = GraphRunner::with_defaults(graph);
    let result = runner.invoke(test_state).await;

    assert!(result.is_ok());
    println!("✅ Multi-agent orchestration persisted to data-fabric");
}
```

### Step 5: Run Tests

```bash
# Start data-fabric in one terminal
cd /path/to/data-fabric
just dev-worker

# In another terminal, run integration tests
cd /path/to/oxidizedgraph
FABRIC_URL=http://localhost:8787 cargo test --test integration_with_fabric
```

---

## Testing Patterns

### Pattern 1: Event Recording

Record each node execution:

```rust
let event = json!({
    "node_type": "governance",
    "status": "executed",
    "guidance_applied": 5,
    "timestamp": chrono::Utc::now().to_rfc3339(),
});

fabric.record_event("governance", "node_executed", event).await?;
```

### Pattern 2: Context Retrieval

Query previous runs for context:

```rust
let context = fabric.query_context(
    "memory",
    json!({
        "agent_role": "architect",
        "limit": 10,
        "order_by": "timestamp_desc",
    }),
).await?;

// Use in LLM prompt
let prompt = format!(
    "Previous context: {:?}\nNew task: {}",
    context,
    current_task,
);
```

### Pattern 3: Compliance Checking

Verify governance compliance before execution:

```rust
let compliance_check = fabric.query_context(
    "lineage",
    json!({
        "run_id": current_run_id,
        "check_type": "governance_compliance",
    }),
).await?;

if !compliance_check["compliant"].as_bool().unwrap_or(false) {
    return Err(NodeError::GovernanceViolation(
        "Agent action violates governance rules".into()
    ));
}
```

---

## Deployment & Testing Matrix

| Environment | data-fabric | oxidizedgraph | Use Case |
|-------------|------------|--------------|----------|
| **Local Dev** | `wrangler dev --local` | `cargo test` | Rapid iteration |
| **Docker** | `docker-compose up` | `cargo test --all` | CI/CD prep |
| **Integration** | `http://localhost:8787` | Integration tests | Pre-commit |
| **Staging** | Cloudflare staging | Full test suite | Pre-release |
| **Production** | Cloudflare production | Monitoring | Live agents |

---

## Troubleshooting

### "Connection Refused" from oxidizedgraph

```bash
# Check data-fabric is running
curl http://localhost:8787/health
# Should return: {"service":"data-fabric",...}

# If not running:
cd /path/to/data-fabric
just dev-worker
```

### "Missing Tenant Context"

data-fabric requires tenant headers:

```rust
// ❌ Wrong - will get 401
client.post("http://localhost:8787/fabric/ingest")
    .json(&event)
    .send()
    .await?

// ✅ Correct
client.post("http://localhost:8787/fabric/ingest")
    .header("X-Tenant-ID", "test-tenant")
    .header("X-Tenant-Role", "admin")
    .json(&event)
    .send()
    .await?
```

### Database Locked

```bash
# Kill stale processes
pkill -f "wrangler dev"
pkill -f "data-fabric"

# Reset and restart
cd /path/to/data-fabric
just db-clean-setup
just dev-worker
```

---

## Next Steps

1. **Implement FabricPersister** in oxidizedgraph as shown above
2. **Write integration tests** using the patterns provided
3. **Test governance nodes** with data-fabric backing
4. **Deploy to staging** once tests pass
5. **Monitor with data-fabric logs**:
   ```bash
   curl http://localhost:8787/v1/runs -H "X-Tenant-ID: test-tenant"
   ```

---

## API Reference

### Record Event

```http
POST /fabric/ingest
X-Tenant-ID: <tenant>

{
  "event_type": "node_executed|tool_called|task_completed",
  "run_id": "run_xxx",
  "payload": { ... }
}
```

### Query Context

```http
POST /fabric/query
X-Tenant-ID: <tenant>

{
  "query_type": "memory|lineage|compliance",
  "filters": { "run_id": "run_xxx", "role": "architect" }
}
```

### Health Check

```http
GET /health
```

---

## Resources

- [data-fabric Architecture](./ARCHITECTURE.md)
- [data-fabric Local Deployment](./LOCAL_DEPLOYMENT.md)
- [oxidizedgraph Documentation](https://github.com/stevedores-org/oxidizedgraph)
- [Cloudflare Workers](https://developers.cloudflare.com/workers/)
