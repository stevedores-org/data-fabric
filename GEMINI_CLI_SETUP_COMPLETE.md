# Gemini CLI Integration — Complete Setup ✅

## What Was Configured

### MCP Server for Gemini CLI (AI-Driven Testing)

**Location**: `/path/to/data-fabric/mcp-server/`

**Files**:
```
mcp-server/
├── data-fabric-mcp.js                    (MCP server implementation - 200+ lines)
├── package.json                          (Dependencies: @modelcontextprotocol/sdk)
├── gemini-extension.json                 (Gemini CLI configuration)
├── README.md                             (Full MCP server documentation - 400+ lines)
└── GEMINI_CLI_INTEGRATION.md             (Quick integration guide - 300+ lines)
```

## How It Works

```
┌─────────────────────────────────────────┐
│  Gemini CLI (cousin-cli)                │
│  "Test data-fabric workflow"            │
└──────────────────┬──────────────────────┘
                   │
              MCP Protocol
                   ↓
       ┌──────────────────────────┐
       │ data-fabric-mcp Server   │
       │                          │
       │ Tools:                   │
       │ • health_check           │
       │ • create_run             │
       │ • ingest_event           │
       │ • query_context          │
       │ • list_runs              │
       │ • provision_tenant       │
       └──────────┬───────────────┘
                  │ HTTP
                  ↓
       ┌──────────────────────┐
       │ data-fabric Worker   │
       │ (localhost:8787)     │
       └──────────────────────┘
```

## Quick Start (3 Steps)

### Step 1: Start data-fabric

```bash
cd /path/to/data-fabric
just dev-worker
# ✨ Listening on http://localhost:8787
```

### Step 2: Register MCP Server with Gemini CLI

```bash
cd /path/to/data-fabric/mcp-server

# Install dependencies
npm install

# Register with Gemini CLI
gemini extensions add .

# Verify
gemini extensions list
# Should see: data-fabric-mcp
```

### Step 3: Start Testing with Gemini

```bash
# Open Gemini CLI
gemini

# Test data-fabric:
> Test if data-fabric is healthy

# Gemini will:
# 1. Call health_check tool
# 2. Parse response
# 3. Report status

# Run full workflow:
> Create a run, ingest 3 events, then query the context
```

## Available Tools (6 total)

| Tool | Purpose | Example |
|------|---------|---------|
| **health_check** | Verify data-fabric is running | "Check if data-fabric is healthy" |
| **create_run** | Create execution tracking run | "Create a run for governance_test" |
| **ingest_event** | Record workflow events | "Ingest a node_executed event" |
| **query_context** | Retrieve historical context | "Query memory from recent runs" |
| **list_runs** | View all runs | "Show me the last 10 runs" |
| **provision_tenant** | Create new tenant | "Provision tenant 'acme-corp'" |

## Example Testing Scenarios

### Test 1: Health Check
```bash
gemini> Check if data-fabric is healthy and report status
```

### Test 2: Complete Workflow
```bash
gemini> Create a new run, ingest 3 test events, \
         then query context to verify events were recorded
```

### Test 3: Governance Validation
```bash
gemini> Simulate a governance workflow: create run, \
         ingest governance events, verify compliance rules applied
```

### Test 4: Integration Test
```bash
gemini> Test oxidizedgraph + data-fabric integration: \
         create run → execute governance node → ingest → verify
```

## Why This Is Powerful

### Before (Manual Testing)
```bash
# Manual curl commands
curl -X POST http://localhost:8787/v1/runs \
  -H "X-Tenant-ID: test" \
  -d '{"spec_sha": "abc123", "flow": "test"}'

# Parse response manually
# Write more curl commands
# Verify manually
# Takes 10+ minutes
```

### After (AI-Driven Testing)
```bash
gemini> Run a complete workflow test and report any issues
# Gemini orchestrates everything automatically
# Takes 30 seconds
# Full report generated
```

## Integration with oxidizedgraph

Test the full orchestration stack:

```bash
# Terminal 1: data-fabric
cd /path/to/data-fabric && just dev-worker

# Terminal 2: oxidizedgraph
cd /path/to/oxidizedgraph && FABRIC_URL=http://localhost:8787 cargo test

# Terminal 3: Gemini testing
gemini> Execute an orchestration workflow through oxidizedgraph that:
         1. Creates a run in data-fabric
         2. Executes governance node
         3. Records output in data-fabric
         4. Verifies compliance
```

## Configuration

### Environment Variables

```bash
# Point to different data-fabric instance
export FABRIC_URL="http://localhost:8787"

# Set tenant
export TENANT_ID="test-tenant"
export TENANT_ROLE="admin"

# Start Gemini CLI
gemini
```

### Using Gemini API Directly

For programmatic testing:

```bash
export GOOGLE_API_KEY="your-gemini-api-key"

# Use Gemini API to test data-fabric
# The MCP tools will be available to the Gemini model
```

## What You Get

✅ **AI-driven testing** — Gemini automatically orchestrates complex workflows
✅ **Natural language** — Ask Gemini what you want to test, not how to test it
✅ **Full orchestration** — Gemini coordinates multiple API calls intelligently
✅ **Report generation** — Get structured test reports
✅ **Compliance validation** — Verify governance rules are enforced
✅ **Integration testing** — Test full orchestration stack (oxidizedgraph + data-fabric)

## Next Steps

1. **Ensure data-fabric is running**: `just dev-worker`
2. **Install MCP dependencies**: `cd mcp-server && npm install`
3. **Register with Gemini**: `gemini extensions add ./mcp-server`
4. **Start testing**: `gemini` then ask it to test

## Files Summary

Total lines of code/documentation:

```
data-fabric-mcp.js                 ~200 lines  (MCP server)
package.json                       ~35 lines   (dependencies)
gemini-extension.json              ~30 lines   (Gemini config)
README.md                          ~400 lines  (Full documentation)
GEMINI_CLI_INTEGRATION.md          ~300 lines  (Integration guide)
────────────────────────────────────────
Total                             ~965 lines
```

## Support

### Troubleshooting

**"Extension not found"**
```bash
gemini extensions add /path/to/data-fabric/mcp-server
```

**"Tool execution failed"**
```bash
# Check data-fabric is running
curl http://localhost:8787/health

# Check MCP server logs
ps aux | grep data-fabric-mcp
```

**"Cannot connect"**
```bash
# Verify FABRIC_URL
echo $FABRIC_URL

# Should be http://localhost:8787 for local dev
export FABRIC_URL="http://localhost:8787"
```

## Resources

- [Gemini CLI Documentation](https://geminicli.com/docs/)
- [Model Context Protocol Spec](https://modelcontextprotocol.io/)
- [data-fabric Architecture](./docs/ARCHITECTURE.md)
- [oxidizedgraph Integration](./docs/INTEGRATION_OXIDIZEDGRAPH.md)

---

**AI-driven testing of data-fabric is now ready.** 🤖

Test with Gemini, improve faster! 🚀
