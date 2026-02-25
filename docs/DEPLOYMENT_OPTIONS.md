# Data Fabric — Deployment Options Guide

Choose the deployment method that works best for your workflow.

## TL;DR - Quick Comparison

| Method | Time | Rust Preference | TypeScript/Bun | Best For |
|--------|------|-----------------|-----------------|----------|
| **Local Native** | 5 min | ✅ ✅ ✅ | ✅ | Development |
| **OCI Containers** (Podman) | 10 min | ✅ ✅ | ✅ | Testing, CI/CD |
| **Docker Compose** | 10 min | ✅ | ✅ | Full stack testing |
| **Cloudflare Remote** | 15 min | ✅ | ✅ | Production |

---

## Option 1: Local Native Development (Recommended for Dev)

### Pros
- ✅ Fastest iteration
- ✅ Full Rust toolchain access
- ✅ Direct debugging
- ✅ Hot-reload working

### Cons
- ⚠️ Requires local Rust setup
- ⚠️ Port conflicts possible
- ⚠️ Platform-specific (macOS/Linux/WSL)

### Setup

```bash
cd /path/to/data-fabric

# One-liner
just setup-local

# Or manually
cargo test --lib
bunx wrangler dev --local

# Test
curl http://localhost:8787/health
```

**Use this for**: Daily development, quick testing, debugging

---

## Option 2: OCI Container (Podman or Docker)

### Pros
- ✅ Isolated environment
- ✅ Podman is rootless & more secure
- ✅ Works in restricted environments
- ✅ Portable across machines
- ✅ **Supports both Rust and TypeScript/Bun testing**

### Cons
- ⚠️ Slightly slower than native
- ⚠️ Container overhead

### Setup with Podman (Recommended)

```bash
cd /path/to/data-fabric

# Build OCI image
podman build -f Containerfile -t data-fabric:latest .

# Run container (rootless)
podman run -d \
  --name data-fabric \
  -p 8787:8787 \
  -v data-fabric-db:/data \
  data-fabric:latest

# Test
curl http://localhost:8787/health

# View logs
podman logs -f data-fabric

# Stop
podman stop data-fabric
```

### Setup with Docker

```bash
# Build image
docker build -f Containerfile -t data-fabric:latest .

# Run container
docker run -d \
  --name data-fabric \
  -p 8787:8787 \
  -v data-fabric-db:/data \
  data-fabric:latest

# Test
curl http://localhost:8787/health
```

**Use this for**: CI/CD pipelines, team environments, production-like testing

---

## Option 3: Docker Compose (Full Stack)

### Pros
- ✅ One-command setup
- ✅ Multi-service orchestration
- ✅ Volume persistence
- ✅ Network isolation

### Cons
- ⚠️ Requires docker-compose or podman-compose
- ⚠️ Slower startup than native

### Setup

```bash
cd /path/to/data-fabric

# Start all services (D1, KV, worker)
docker-compose up -d

# Check status
docker-compose ps

# View logs
docker-compose logs -f worker

# Test
curl http://localhost:8787/health

# Stop
docker-compose down

# Full cleanup
docker-compose down -v
```

**Use this for**: Full integration testing, database verification, multi-service testing

---

## Option 4: Cloudflare Remote (Production)

### Pros
- ✅ True Cloudflare environment
- ✅ Real D1, R2, KV
- ✅ Global distribution
- ✅ Production-ready

### Cons
- ⚠️ Requires Cloudflare account
- ⚠️ Account ID & API keys needed
- ⚠️ Network latency

### Setup

```bash
# 1. Authenticate
bunx wrangler login

# 2. Update wrangler.toml with your account_id
# Edit: wrangler.toml
# account_id = "YOUR_ACCOUNT_ID"
# database_id = "YOUR_DB_ID"

# 3. Deploy
bunx wrangler deploy

# 4. View logs
bunx wrangler tail

# Test
curl https://data-fabric.stevedores.org/health
```

**Use this for**: Production deployments, real workloads, performance validation

---

## Decision Matrix

### "I want to develop locally quickly"
→ **Option 1: Local Native**
```bash
just setup-local && just dev-worker
```

### "I want to test with oxidizedgraph"
→ **Option 2: OCI Container (Podman)**
```bash
podman build -f Containerfile -t data-fabric:latest .
podman run -p 8787:8787 data-fabric:latest
# In another terminal:
# FABRIC_URL=http://localhost:8787 cargo test
```

### "I want full stack testing with persistence"
→ **Option 3: Docker Compose**
```bash
docker-compose up -d
# Test full stack with database
```

### "I want to deploy to production"
→ **Option 4: Cloudflare Remote**
```bash
bunx wrangler login
bunx wrangler deploy
```

---

## Testing Integration with oxidizedgraph

Each deployment option works with oxidizedgraph:

### With Local Native
```bash
# Terminal 1: data-fabric
cd /path/to/data-fabric
just dev-worker

# Terminal 2: oxidizedgraph tests
cd /path/to/oxidizedgraph
FABRIC_URL=http://localhost:8787 cargo test --test integration_with_fabric
```

### With OCI Container
```bash
# Terminal 1: Podman
podman run -p 8787:8787 data-fabric:latest

# Terminal 2: oxidizedgraph tests
cd /path/to/oxidizedgraph
FABRIC_URL=http://localhost:8787 cargo test --test integration_with_fabric
```

### With Docker Compose
```bash
# Start full stack
docker-compose up -d

# Run integration tests
cd /path/to/oxidizedgraph
FABRIC_URL=http://localhost:8787 cargo test --test integration_with_fabric

# Cleanup
docker-compose down
```

---

## Platform-Specific Notes

### macOS (M1/M2/M3)
- ✅ **Recommended**: Local native or Podman
- ✅ All options work
- ⚠️ Docker Desktop may require more resources

```bash
# Native (fastest on Apple Silicon)
just dev-worker

# Or Podman (more secure)
podman build -f Containerfile -t data-fabric:latest .
podman run -p 8787:8787 data-fabric:latest
```

### Linux
- ✅ **Recommended**: Podman (native support)
- ✅ All options work
- ⚠️ May require `podman.socket` for compose

```bash
# Check if podman socket is running
systemctl --user status podman.socket

# If not running, start it
systemctl --user enable --now podman.socket
```

### Windows (WSL2)
- ✅ **Recommended**: WSL2 + local native
- ✅ Docker Desktop in WSL2
- ⚠️ Podman support improving

```bash
# In WSL2 terminal
cd /path/to/data-fabric
just dev-worker

# Or use Docker
docker build -f Containerfile -t data-fabric:latest .
```

---

## Performance Comparison

| Operation | Native | Container | Compose |
|-----------|--------|-----------|---------|
| **Build** | ~30s | ~60s | ~60s |
| **Startup** | ~1s | ~3s | ~5s |
| **Health Check** | <1ms | <5ms | <5ms |
| **Request** | <5ms | <10ms | <10ms |
| **Memory** | ~200MB | ~400MB | ~600MB |

---

## Troubleshooting

### Port 8787 Already in Use

```bash
# Find process using port
lsof -i :8787

# Kill process
kill -9 <PID>

# Or use different port
podman run -p 9000:8787 data-fabric:latest
curl http://localhost:9000/health
```

### Container Build Fails

```bash
# Option 1: Clean build
podman build --no-cache -f Containerfile .

# Option 2: Use native instead
just dev-worker

# Option 3: Check logs
podman build -f Containerfile . --log-level debug
```

### Connection Refused from oxidizedgraph

```bash
# Verify data-fabric is running
curl http://localhost:8787/health

# Check network connectivity
podman network ls
podman inspect data-fabric

# Use docker.io network
docker-compose exec -T worker curl http://localhost:8787/health
```

---

## Next Steps

1. **Choose your option**: Pick one from above based on your workflow
2. **Follow setup**: Execute the commands for that option
3. **Test health**: `curl http://localhost:8787/health`
4. **Integrate with oxidizedgraph**: See [INTEGRATION_OXIDIZEDGRAPH.md](./INTEGRATION_OXIDIZEDGRAPH.md)
5. **Join team**: Add to shared development setup

---

## References

- [Local Development Guide](./LOCAL_DEPLOYMENT.md)
- [OCI Container Deployment](./OCI_DEPLOYMENT.md)
- [Integration with oxidizedgraph](./INTEGRATION_OXIDIZEDGRAPH.md)
- [Architecture](./ARCHITECTURE.md)
