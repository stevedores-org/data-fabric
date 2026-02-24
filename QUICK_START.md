# Data Fabric — Quick Start Checklist

## ✅ Pre-Flight Checklist

- [ ] Rust installed: `rustc --version`
- [ ] Cargo installed: `cargo --version`
- [ ] Bun installed: `bun --version`
- [ ] Just installed: `just --version`
- [ ] Podman or Docker available: `podman --version` or `docker --version`

## ✅ Choose Your Path

**Path A: Local Development (Fastest - Recommended)**
```bash
cd /path/to/data-fabric
just setup-local && just dev-worker
# ✨ Worker listening on http://localhost:8787
```

**Path B: OCI Container (Podman)**
```bash
cd /path/to/data-fabric
podman build -f Containerfile -t data-fabric:latest .
podman run -p 8787:8787 data-fabric:latest
```

**Path C: Full Stack (Docker Compose)**
```bash
cd /path/to/data-fabric
docker-compose up -d
```

## ✅ Verify It Works

```bash
# Test health
curl http://localhost:8787/health

# Should return:
# {"service":"data-fabric","status":"ok",...}
```

## ✅ Set Up Gemini CLI Testing (Optional but Recommended)

```bash
# 1. Register MCP server
cd /path/to/data-fabric/mcp-server
npm install
gemini extensions add .

# 2. Start testing
gemini

# Ask Gemini:
# > Test if data-fabric is healthy
# > Create a run and ingest some events
# > Run a complete workflow test
```

## ✅ Test with oxidizedgraph

```bash
# Terminal 1: data-fabric
cd /path/to/data-fabric
just dev-worker

# Terminal 2: oxidizedgraph
cd /path/to/oxidizedgraph
FABRIC_URL=http://localhost:8787 cargo test --test integration_with_fabric

# Terminal 3: Gemini (optional)
gemini
# > Test oxidizedgraph + data-fabric integration
```

## 📚 Documentation

| Document | Purpose |
|----------|---------|
| `README_DEPLOYMENT.md` | Overview & quick start |
| `docs/LOCAL_DEPLOYMENT.md` | Local development guide |
| `docs/OCI_DEPLOYMENT.md` | Container deployment |
| `docs/DEPLOYMENT_OPTIONS.md` | Compare all approaches |
| `docs/INTEGRATION_OXIDIZEDGRAPH.md` | oxidizedgraph integration |
| `mcp-server/README.md` | Gemini CLI MCP server |

## 🔧 Common Tasks

```bash
# Run tests
just test

# Check code quality
just check

# Format code
just fmt

# Start dev server
just dev-worker

# Reset database
just db-clean-setup

# View all tasks
just help
```

## ⚠️ Troubleshooting

**Port 8787 already in use?**
```bash
lsof -i :8787
kill -9 <PID>
```

**Database locked?**
```bash
pkill -f "wrangler dev"
just db-reset
just dev-worker
```

**Gemini extension not found?**
```bash
gemini extensions add /path/to/data-fabric/mcp-server
```

## 🎯 Next Steps

1. **Choose path** (A, B, or C above)
2. **Start server** (see your chosen path)
3. **Verify health**: `curl http://localhost:8787/health`
4. **Run tests**: `just test`
5. **Test with Gemini** (optional): `gemini` → ask it to test
6. **Integrate with oxidizedgraph** (see INTEGRATION_OXIDIZEDGRAPH.md)

---

**Time to get started**: 5 minutes ⏱️

Start with Path A for fastest results!
