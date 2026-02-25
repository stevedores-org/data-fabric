# Data Fabric — Local Deployment Guide

> **Status**: Ready for local development & testing with oxidizedgraph

This guide shows how to deploy data-fabric locally for development, testing, and integration with oxidizedgraph.

## Overview

data-fabric is a Cloudflare Worker-based service built in Rust (compiled to WASM). For local development, we use:

| Component | Local Setup | Purpose |
|-----------|------------|---------|
| **Worker** | `wrangler dev --local` | HTTP API endpoints |
| **D1 (SQLite)** | `.wrangler/state/v3/d1/` | Metadata & structured data |
| **KV** | In-memory (wrangler) | Policies, config cache |
| **R2** | Mocked by wrangler | Artifact storage |
| **Queues** | In-memory (wrangler) | Event bus (optional) |
| **Tests** | `cargo test --lib` | Unit tests (70 passing) |

## Quick Start (5 minutes)

### Prerequisites

```bash
# Install just (macOS)
brew install just

# Or use cargo
cargo install just

# Verify tools
rustc --version      # 1.75+
cargo --version      # 1.75+
bun --version        # 1.0+
```

### Setup

```bash
cd /path/to/data-fabric

# One-command setup
just setup-local

# Run tests
just test

# Start development server
just dev-worker
```

That's it! The worker is now available at `http://localhost:8787`

---

## Detailed Setup

### 1. Install Wrangler (via Bun)

Wrangler is the Cloudflare Workers CLI tool. We use `bunx` to install it without global npm:

```bash
cd /path/to/data-fabric

# This automatically fetches wrangler
bunx wrangler --version
# Output: 4.68.0
```

### 2. Build WASM for Local Development

```bash
# Install worker-build (Cloudflare's build tool)
cargo install worker-build

# Build for local dev (optimized)
worker-build --release

# Output: build/worker/shim.mjs (ES module)
```

### 3. Set Up Local D1 Database

D1 is Cloudflare's SQLite wrapper. Locally, it creates a real SQLite database:

```bash
# Create local database
bunx wrangler d1 create data-fabric --local

# This creates: .wrangler/state/v3/d1/data-fabric.sqlite

# Apply migrations
bunx wrangler d1 migrations apply data-fabric --local

# Verify
bunx wrangler d1 execute data-fabric --local -- "SELECT count(*) FROM runs;"
```

### 4. Start Development Server

```bash
# Terminal 1: Watch & rebuild on changes
cargo watch -x "build --target wasm32-unknown-unknown --release"

# Terminal 2: Run dev server
bunx wrangler dev --local

# Output:
# ⛅ wrangler (dev)
# ✨ Listening on http://localhost:8787
# [b] open a browser, [d] open Devtools, [c] clear console, [x] to exit
```

The worker is now live with hot-reload! Changes to Rust code recompile and redeploy automatically.

### 5. Test Local Deployment

```bash
# Health check
curl http://localhost:8787/health

# Response:
# {"service":"data-fabric","status":"ok","mission":"velocity-for-autonomous-agent-builders"}

# Create a run (requires tenant auth headers)
curl -X POST http://localhost:8787/v1/runs \
  -H "Content-Type: application/json" \
  -H "X-Tenant-ID: test-tenant" \
  -H "X-Tenant-Role: admin" \
  -d '{"spec_sha": "abc123", "flow": "test-flow"}'

# Response:
# {"id":"run_xxxxx","status":"created"}
```

---

## Using with oxidizedgraph

### Scenario: Test orchestrationgraph Nodes with data-fabric Backend

data-fabric provides persistence and context for oxidizedgraph workflows:

#### 1. Start data-fabric locally

```bash
# Terminal 1: data-fabric
cd /path/to/data-fabric
just dev-worker

# ✨ Listening on http://localhost:8787
```

#### 2. Configure oxidizedgraph for data-fabric

In oxidizedgraph tests, point nodes to local data-fabric:

```rust
// In oxidizedgraph tests
#[tokio::test]
async fn test_orchestration_with_data_fabric() {
    // Point to local data-fabric
    let fabric_url = std::env::var("FABRIC_URL")
        .unwrap_or_else(|_| "http://localhost:8787".to_string());

    // Create a governance node that persists to data-fabric
    let gov_node = GovernanceNode::new(
        FabricPersister::new(&fabric_url),
        AgentRole::Architect,
    );

    let state = AgentState::default();
    let result = gov_node.execute(state).await;

    assert!(result.is_ok());
    // Verify state was persisted to data-fabric
}
```

#### 3. Run oxidizedgraph Integration Tests

```bash
cd /path/to/oxidizedgraph

# Start data-fabric in background
FABRIC_URL=http://localhost:8787 cargo test --test integration_with_fabric

# Or use the helper target
just test-with-fabric
```

---

## API Reference

### Health Check

```http
GET /health
```

**Response:**
```json
{
  "service": "data-fabric",
  "status": "ok",
  "mission": "velocity-for-autonomous-agent-builders"
}
```

### Run Management

#### Create Run

```http
POST /v1/runs
X-Tenant-ID: <tenant-id>
X-Tenant-Role: admin|member

{
  "spec_sha": "sha256:abc123...",
  "flow": "orchestration-flow-name"
}
```

**Response:**
```json
{
  "id": "run_1c4a5b9e2f...",
  "status": "created"
}
```

#### Ingest Event

```http
POST /fabric/ingest
X-Tenant-ID: <tenant-id>

{
  "event_type": "tool_called|tool_returned|task_started",
  "run_id": "run_xxx",
  "payload": { ... }
}
```

#### Query Context

```http
POST /fabric/query
X-Tenant-ID: <tenant-id>

{
  "query_type": "memory|lineage|artifact",
  "filters": { "run_id": "run_xxx" }
}
```

---

## Development Workflow

### Watch & Rebuild

```bash
# Automatically rebuild on file changes
cargo watch -x "build --target wasm32-unknown-unknown --release"
```

### Run Tests with Coverage

```bash
# Unit tests (70 tests)
just test

# Integration tests (with running worker)
just test-integration

# Specific test
cargo test policy::tests::risk_classification_matches_expected
```

### Database Management

```bash
# View database state
bunx wrangler d1 shell data-fabric --local

# Then at prompt:
SELECT * FROM runs LIMIT 5;
SELECT count(*) FROM events;
.exit

# Reset database
just db-reset

# Full clean setup
just db-clean-setup
```

### Code Quality

```bash
# Format
just fmt

# Lint
just lint

# All checks
just check
```

---

## Troubleshooting

### Worker Won't Start

```bash
# Check logs
tail -f /tmp/data-fabric-dev.log

# Common issues:
# - Port 8787 in use: lsof -i :8787
# - WASM build failed: rm -rf build/ && cargo clean && worker-build --release
# - D1 locked: rm .wrangler/state/v3/d1/.lock
```

### Database Locked

```bash
# If you see "database is locked" errors:
pkill -f "wrangler dev"
pkill -f "data-fabric"
rm -f .wrangler/state/v3/d1/.lock
just dev-worker
```

### Migration Failures

```bash
# Check migration files
ls migrations/

# Apply specific migration
bunx wrangler d1 execute data-fabric --local < migrations/0001_ws2_domain_model.sql

# Or reset and reapply
just db-clean-setup
```

### Tests Not Compiling

```bash
# Clear cache
cargo clean
cargo fetch

# Rebuild
cargo test --lib --no-run

# Run
cargo test --lib
```

---

## Performance Notes

### Local vs. Remote

| Metric | Local | Remote |
|--------|-------|--------|
| **Startup** | ~1-2s | ~100ms |
| **Latency** | <10ms | 50-200ms |
| **DB Queries** | SQLite (disk) | D1 (replicated) |
| **Throughput** | Unlimited | Rate-limited |

For development, local is excellent. For production simulation, use remote.

---

## Deployment to Cloudflare

### Prerequisites

```bash
# Create Cloudflare account
# Get API token from: https://dash.cloudflare.com/profile/api-tokens

# Authenticate
bunx wrangler login

# This creates ~/.wrangler/config/default.toml with credentials
```

### Deploy to Production

```bash
# Update wrangler.toml with your account_id and database_id
# Then:
just deploy-prod

# View logs
just logs-remote
```

### Environment Variables

```bash
# In wrangler.toml or via CLI:
bunx wrangler secret put DATABASE_URL
bunx wrangler secret put API_KEY

# Verify
bunx wrangler secret list
```

---

## Integration with CI/CD

### GitHub Actions Example

```yaml
name: Test data-fabric

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: denoland/setup-deno@v1
      - name: Install Rust
        run: rustup target add wasm32-unknown-unknown
      - name: Run tests
        run: cargo test --lib
      - name: Build WASM
        run: |
          cargo install worker-build
          worker-build --release
      - name: Integration test
        run: just test-integration
```

---

## Next Steps

1. **Start development**: `just dev-worker`
2. **Run tests**: `just test`
3. **Integrate with oxidizedgraph**: See "Using with oxidizedgraph" section above
4. **Deploy to Cloudflare**: `just deploy-prod` (after wrangler auth)

---

## Resources

- [Cloudflare Workers Documentation](https://developers.cloudflare.com/workers/)
- [D1 Documentation](https://developers.cloudflare.com/d1/)
- [Wrangler CLI](https://developers.cloudflare.com/workers/wrangler/)
- [data-fabric ARCHITECTURE.md](./ARCHITECTURE.md)
- [oxidizedgraph Repository](https://github.com/stevedores-org/oxidizedgraph)
