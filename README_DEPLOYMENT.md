# Data Fabric — Local Deployment & Testing Setup

> **Status**: ✅ Ready for local development, testing, and oxidizedgraph integration

This document provides a **quick start guide** for establishing local data-fabric deployment to test with oxidizedgraph.

## 🎯 What You Get

- ✅ **Rust + WASM**: data-fabric compiles to WebAssembly
- ✅ **TypeScript/Bun**: wrangler dev server for local testing
- ✅ **OCI Containers**: Supports Podman, Docker, Buildah
- ✅ **Full Stack Testing**: docker-compose with D1, KV, R2
- ✅ **70 Passing Tests**: Unit test suite fully integrated
- ✅ **oxidizedgraph Integration**: Ready for node testing

---

## ⚡ 30-Second Quick Start

```bash
cd /path/to/data-fabric

# Option A: Local native (fastest)
just setup-local && just dev-worker

# Option B: OCI container (Podman)
podman build -f Containerfile -t data-fabric:latest .
podman run -p 8787:8787 data-fabric:latest

# Option C: Full stack (Docker Compose)
docker-compose up -d

# Test health
curl http://localhost:8787/health
```

Pick **one option** based on your workflow (see table below).

---

## 🛠️ Choosing Your Setup

| Workflow | Recommended | Command | Time |
|----------|-------------|---------|------|
| **Daily development** | Local native | `just dev-worker` | 1s |
| **CI/CD testing** | OCI (Podman) | `podman build -f Containerfile .` | 60s |
| **Full integration** | Docker Compose | `docker-compose up -d` | 5s |
| **Production deploy** | Cloudflare | `bunx wrangler deploy` | 30s |

---

## 📋 Setup Verification

Verify your environment is ready:

```bash
cd /path/to/data-fabric

# Run verification script
bash .setup-verification.sh

# Output:
# ✅ Rust: rustc 1.93.1
# ✅ Cargo: cargo 1.93.1
# ✅ Bun: v1.3.9
# ✅ Just: just 1.46.0
# ✅ Podman: podman 5.8.0 (rootless)
# ✅ All 70 unit tests passing
# ✅ Setup verification complete!
```

---

## 🚀 Option A: Local Native (Recommended for Dev)

**Best for**: Daily development, rapid iteration, debugging

### Setup (5 minutes)

```bash
cd /path/to/data-fabric

# Install dependencies & apply migrations
just setup-local

# Output:
# 🔧 Setting up local data-fabric development environment...
# 📦 Creating local D1 database...
# 🗂️  Applying migrations to local database...
# ✅ Local development environment ready!
```

### Start Development Server

```bash
# Terminal 1: Development server (with hot-reload)
just dev-worker

# Output:
# ✨ Listening on http://localhost:8787
# [b] open a browser, [d] open Devtools, [c] clear console, [x] to exit
```

### Test It

```bash
# Terminal 2: Test health endpoint
curl http://localhost:8787/health

# Response:
# {"service":"data-fabric","status":"ok","mission":"velocity-for-autonomous-agent-builders"}
```

### Run Tests

```bash
# Terminal 2: Run unit tests
just test

# Output:
# test result: ok. 70 passed; 0 failed
```

### Clean Up

```bash
# Stop development server: Press Ctrl+C
# Reset database: just db-reset
# Full clean: just db-clean-setup
```

---

## 🐳 Option B: OCI Container (Podman or Docker)

**Best for**: CI/CD, team environments, isolated testing

### Setup (10 minutes)

#### Using Podman (Recommended - Rootless)

```bash
cd /path/to/data-fabric

# Build OCI image
podman build -f Containerfile -t data-fabric:latest .

# Output:
# ...
# COMMIT data-fabric:latest
# ✅ Successfully built data-fabric:latest

# Run container
podman run -d \
  --name data-fabric \
  -p 8787:8787 \
  -v data-fabric-db:/data \
  data-fabric:latest

# Output:
# 1a2b3c4d5e6f7g8h9i0j1k2l3m4n5o6p
```

#### Using Docker

```bash
cd /path/to/data-fabric

# Build image
docker build -f Containerfile -t data-fabric:latest .

# Run container
docker run -d \
  --name data-fabric \
  -p 8787:8787 \
  -v data-fabric-db:/data \
  data-fabric:latest
```

### Test It

```bash
# Check health
curl http://localhost:8787/health

# View logs
podman logs -f data-fabric        # Podman
docker logs -f data-fabric        # Docker
```

### Clean Up

```bash
# Stop container
podman stop data-fabric
podman rm data-fabric

# Remove image
podman rmi data-fabric:latest
```

---

## 📦 Option C: Docker Compose (Full Stack)

**Best for**: Complete integration testing with database verification

### Setup (5 minutes)

```bash
cd /path/to/data-fabric

# Start all services
docker-compose up -d

# Output:
# Creating data-fabric-sqlite-init ... done
# Creating data-fabric-worker       ... done

# Check status
docker-compose ps

# Output:
# NAME                         STATUS
# data-fabric-sqlite-init      Exited
# data-fabric-worker           Up (healthy)
```

### Test It

```bash
# Health check
curl http://localhost:8787/health

# View logs
docker-compose logs -f worker
```

### Full Stack Operations

```bash
# Check database
sqlite3 .wrangler/state/v3/d1/data-fabric.sqlite
> SELECT count(*) FROM runs;

# Create test tenant
curl -X POST http://localhost:8787/v1/tenants/provision \
  -H "Content-Type: application/json" \
  -H "X-Tenant-ID: admin" \
  -d '{"tenant_id":"test-org","display_name":"Test Organization"}'
```

### Clean Up

```bash
# Stop all services
docker-compose down

# Remove volumes (database)
docker-compose down -v
```

---

## 🔗 Integration with oxidizedgraph

Once data-fabric is running locally, integrate it with oxidizedgraph:

### 1. Point oxidizedgraph to Local data-fabric

```rust
// In oxidizedgraph tests
let fabric_url = std::env::var("FABRIC_URL")
    .unwrap_or_else(|_| "http://localhost:8787".to_string());

let fabric = Arc::new(FabricPersister::new(
    &fabric_url,
    "test-tenant",
    "run_123",
));
```

### 2. Run Integration Tests

```bash
# Start data-fabric
cd /path/to/data-fabric
just dev-worker

# In another terminal
cd /path/to/oxidizedgraph
FABRIC_URL=http://localhost:8787 cargo test --test integration_with_fabric
```

### 3. Full Example

```bash
# Terminal 1: Start data-fabric
cd /path/to/data-fabric
just dev-worker

# Terminal 2: Run governance + fabric tests
cd /path/to/oxidizedgraph
FABRIC_URL=http://localhost:8787 cargo test test_governance_workflow_with_fabric_persistence

# Output:
# test test_governance_workflow_with_fabric_persistence ... ok
# ✅ Governance execution persisted to data-fabric
```

See [INTEGRATION_OXIDIZEDGRAPH.md](./docs/INTEGRATION_OXIDIZEDGRAPH.md) for full integration guide.

---

## 📊 Architecture Summary

```
┌─────────────────────────────────────────┐
│   Your Development Environment         │
├─────────────────────────────────────────┤
│                                         │
│  ┌──────────────┐  ┌──────────────┐   │
│  │  oxidizedgraph │  │ data-fabric   │   │
│  │     (Rust)   │  │  (WASM+Rust)  │   │
│  └──────────────┘  └──────────────┘   │
│         │                    │          │
│         └────────┬───────────┘          │
│                  │ HTTP                 │
│         ┌────────v────────┐            │
│         │  Localhost:8787 │            │
│         │  (Development)  │            │
│         └────────────────┘            │
│                  │                     │
│         ┌────────v────────┐            │
│         │  D1 (SQLite)    │            │
│         │  KV (Cache)     │            │
│         │  R2 (Artifacts) │            │
│         └────────────────┘            │
└─────────────────────────────────────────┘
```

---

## 🔧 Makefile / Just Targets

### Common Tasks

```bash
# Development
just dev              # Full setup + dev server
just test             # Run 70 unit tests
just test-watch       # Watch & rerun tests
just check            # Linting + formatting
just fmt              # Format code
just lint             # Clippy checks

# Deployment
just dev-worker       # Start dev server
just dev-worker       # Local native (recommended)
just podman-build     # Build OCI image
just docker-build     # Build OCI image
just docker-up        # Start docker-compose
just docker-down      # Stop docker-compose

# Database
just db-seed          # Seed with migrations
just db-reset         # Reset local database
just db-clean-setup   # Reset + reapply migrations

# Cloudflare (Production)
just deploy-prod      # Deploy to Cloudflare
just deploy-staging   # Deploy to staging
just logs-remote      # View Cloudflare logs
```

### Status Check

```bash
just status

# Output:
# === data-fabric Status ===
# Rust version: rustc 1.93.1
# Cargo version: cargo 1.93.1
# Wrangler version: 4.68.0
# ...
```

---

## 🐛 Troubleshooting

### "Port 8787 Already in Use"

```bash
# Find process
lsof -i :8787

# Kill process
kill -9 <PID>

# Or use different port
podman run -p 9000:8787 data-fabric:latest
curl http://localhost:9000/health
```

### "Failed to Start Worker"

```bash
# Check logs
tail -f /tmp/data-fabric-dev.log

# Clean rebuild
rm -rf build/ .wrangler/
cargo clean
just dev-worker
```

### "Database Locked"

```bash
# Kill hanging processes
pkill -f "wrangler dev"
pkill -f "data-fabric"

# Reset and restart
just db-clean-setup
just dev-worker
```

### "Container Build Fails"

```bash
# Clean build without cache
podman build --no-cache -f Containerfile .

# Or use local native instead
just dev-worker
```

---

## 📚 Documentation

- [**LOCAL_DEPLOYMENT.md**](./docs/LOCAL_DEPLOYMENT.md) — Detailed local setup guide
- [**OCI_DEPLOYMENT.md**](./docs/OCI_DEPLOYMENT.md) — OCI standards & container details
- [**INTEGRATION_OXIDIZEDGRAPH.md**](./docs/INTEGRATION_OXIDIZEDGRAPH.md) — oxidizedgraph integration
- [**DEPLOYMENT_OPTIONS.md**](./docs/DEPLOYMENT_OPTIONS.md) — Choose your approach
- [**ARCHITECTURE.md**](./docs/ARCHITECTURE.md) — System architecture

---

## ✅ Verification Checklist

- [ ] Run `.setup-verification.sh` successfully
- [ ] Choose deployment option (A, B, or C)
- [ ] Start data-fabric server
- [ ] Test health endpoint: `curl http://localhost:8787/health`
- [ ] Run unit tests: `just test` (all 70 passing)
- [ ] Create test tenant (if testing with DB)
- [ ] Integrate with oxidizedgraph tests

---

## 🎯 Next Steps

1. **Choose Option A, B, or C** above
2. **Run setup** (5-10 minutes)
3. **Test health** with curl
4. **Run unit tests** to verify
5. **Integrate with oxidizedgraph** using [INTEGRATION_OXIDIZEDGRAPH.md](./docs/INTEGRATION_OXIDIZEDGRAPH.md)
6. **Deploy to Cloudflare** when ready (optional)

---

## 💡 Key Points

- ✅ **Rust-first**: Built in Rust, compiles to WASM
- ✅ **TypeScript/Bun compatible**: Uses bunx wrangler for dev
- ✅ **OCI standard**: Works with Podman, Docker, Buildah
- ✅ **Development ready**: 70 tests passing, hot-reload working
- ✅ **Production capable**: Deploy to Cloudflare via `wrangler deploy`
- ✅ **oxidizedgraph integration**: Persistence layer for governance nodes

---

## 🤝 Support

Questions? Check:
- Troubleshooting section above
- Individual docs/* guides
- `.setup-verification.sh` output

---

**Happy developing! 🎉**
