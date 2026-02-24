# Data Fabric MCP Server

**Use Gemini AI to test and interact with data-fabric** via Model Context Protocol (MCP).

## Overview

This MCP server exposes all data-fabric API operations as tools that Gemini CLI can use. This enables:

- **AI-driven testing**: Ask Gemini "test data-fabric health and report issues"
- **Agentic workflows**: Gemini automatically creates runs, ingests events, queries context
- **Natural language queries**: "Show me the last 10 runs" → automatic API call
- **Integration testing**: Test oxidizedgraph + data-fabric orchestration workflows

```
┌─────────────────────────────────────────┐
│  Gemini CLI (cousin-cli)                │
│  "Test data-fabric workflow"            │
└──────────────────┬──────────────────────┘
                   │
                   │ MCP Protocol
                   ↓
        ┌──────────────────────┐
        │ data-fabric-mcp      │
        │ (This Server)        │
        │                      │
        │ - health_check       │
        │ - create_run         │
        │ - ingest_event       │
        │ - query_context      │
        │ - list_runs          │
        │ - provision_tenant   │
        └──────────┬───────────┘
                   │ HTTP
                   ↓
        ┌──────────────────────┐
        │ data-fabric Worker   │
        │ (localhost:8787)     │
        └──────────────────────┘
```

## Installation

### 1. Prerequisites

```bash
# Ensure you have Gemini CLI (cousin-cli) installed
npm install -g @google/gemini-cli

# Verify
gemini --version
```

### 2. Install MCP Server

```bash
cd /path/to/data-fabric/mcp-server

# Install dependencies
npm install

# Make executable (optional)
chmod +x data-fabric-mcp.js
```

### 3. Register with Gemini CLI

```bash
# Add extension to Gemini CLI
gemini extensions add /path/to/data-fabric/mcp-server

# Verify
gemini extensions list
# Should see: data-fabric-mcp
```

## Usage

### Start data-fabric

```bash
cd /path/to/data-fabric
just dev-worker
# ✨ Listening on http://localhost:8787
```

### Test with Gemini CLI

```bash
# Start an interactive session
gemini

# In the Gemini CLI prompt, ask it to test data-fabric:
> Test the data-fabric health and report status

# Output:
# Using tool: health_check
# Result:
# {
#   "status": 200,
#   "data": {
#     "service": "data-fabric",
#     "status": "ok",
#     "mission": "velocity-for-autonomous-agent-builders"
#   }
# }
```

### Example Workflows

#### Workflow 1: Test Health & Create Run

```bash
gemini

> Check data-fabric health, then create a test run with spec_sha="abc123" and flow="governance_test"

# Gemini will:
# 1. Call health_check
# 2. If healthy, call create_run
# 3. Report results
```

#### Workflow 2: Full Workflow Test

```bash
gemini

> Create a new run, ingest 3 test events (node_executed, tool_called, task_completed), \
> then query the context to verify events were recorded

# Gemini will automatically orchestrate all 5+ API calls
```

#### Workflow 3: Integration with oxidizedgraph

```bash
gemini

> Simulate a governance workflow: create a run, ingest governance-related events, \
> then query to verify compliance rules were applied

# Tests the full orchestration flow
```

## Environment Configuration

### Configuration via Environment Variables

```bash
# Point to different data-fabric instance
export FABRIC_URL="http://production.example.com:8787"
export TENANT_ID="my-org"
export TENANT_ROLE="admin"

# Start server
node data-fabric-mcp.js
```

### Configuration via .env File

Create `.env` in mcp-server directory:

```bash
FABRIC_URL=http://localhost:8787
TENANT_ID=test-tenant
TENANT_ROLE=admin
```

Then run:

```bash
node data-fabric-mcp.js
```

## Available Tools

### 1. health_check

Check if data-fabric is running and healthy.

**Usage:**
```bash
gemini> Check if data-fabric is healthy
```

**Response:**
```json
{
  "status": 200,
  "data": {
    "service": "data-fabric",
    "status": "ok",
    "mission": "velocity-for-autonomous-agent-builders"
  }
}
```

### 2. create_run

Create a new run for tracking orchestration execution.

**Parameters:**
- `spec_sha` (required): SHA256 hash of configuration
- `flow` (required): Workflow name

**Usage:**
```bash
gemini> Create a run with spec_sha="abc123xyz" and flow="governance_workflow"
```

**Response:**
```json
{
  "id": "run_1a2b3c4d5e6f",
  "status": "created"
}
```

### 3. ingest_event

Record an event in a run (node execution, tool calls, etc).

**Parameters:**
- `run_id` (required): Run to record in
- `event_type` (required): "node_executed" | "tool_called" | "task_completed"
- `payload` (optional): Event details

**Usage:**
```bash
gemini> In run run_123, ingest a node_executed event for the governance node \
         with status="success" and guidance_applied=5
```

### 4. query_context

Query context from data-fabric (memory, lineage, artifacts).

**Parameters:**
- `query_type` (required): "memory" | "lineage" | "artifact"
- `run_id` (optional): Filter by specific run

**Usage:**
```bash
gemini> Query memory context for recent runs
```

### 5. list_runs

List all runs for the current tenant.

**Parameters:**
- `limit` (optional): Max results to return

**Usage:**
```bash
gemini> Show me the last 20 runs
```

### 6. provision_tenant

Provision a new tenant (admin only).

**Parameters:**
- `tenant_id` (required): Unique identifier
- `display_name` (required): Human-readable name

**Usage:**
```bash
gemini> Provision a new tenant "my-org" with id "my-org-001"
```

## Advanced: Creating Custom Prompts

Add structured prompts for complex testing workflows:

```bash
gemini> Use the "test-fabric-workflow" prompt to run a complete workflow
```

Edit `gemini-extension.json` to add prompts:

```json
{
  "prompts": [
    {
      "name": "test-fabric-workflow",
      "description": "Run complete workflow: health check → create run → ingest events → query",
      "arguments": [
        {
          "name": "event_count",
          "description": "Number of test events to ingest",
          "required": false
        }
      ]
    }
  ]
}
```

Then in Gemini CLI:

```bash
gemini> Execute the test-fabric-workflow prompt with 5 events
```

## Troubleshooting

### "Cannot connect to data-fabric"

```bash
# Check if data-fabric is running
curl http://localhost:8787/health

# If not, start it
cd /path/to/data-fabric
just dev-worker

# Check FABRIC_URL environment variable
echo $FABRIC_URL

# If needed, set it
export FABRIC_URL="http://localhost:8787"
```

### "Extension not found"

```bash
# Verify registration
gemini extensions list

# Re-add if needed
gemini extensions add /path/to/data-fabric/mcp-server

# Check path is absolute
realpath /path/to/data-fabric/mcp-server
```

### "Tool execution failed"

Check that:
1. data-fabric is running and healthy
2. Correct FABRIC_URL is set
3. Tenant headers (X-Tenant-ID, X-Tenant-Role) are set correctly
4. Request payload matches API expectations

### "Authorization failed"

Most tools require `X-Tenant-ID` and `X-Tenant-Role` headers:

```bash
# Create a test tenant first
gemini> Provision a test tenant with id "test-org" and name "Test Organization"

# Then use that tenant
export TENANT_ID="test-org"
export TENANT_ROLE="admin"
```

## Development

### Test the MCP Server Directly

```bash
# In one terminal, start data-fabric
cd /path/to/data-fabric
just dev-worker

# In another terminal, test the MCP server
cd /path/to/data-fabric/mcp-server
node data-fabric-mcp.js

# Output should show:
# [data-fabric-mcp] Server running
# [data-fabric-mcp] FABRIC_URL: http://localhost:8787
# [data-fabric-mcp] Tools available: 6
```

### Add New Tools

To add a new tool:

1. Add to `tools` array in `data-fabric-mcp.js`:
```javascript
{
  name: "my_tool",
  description: "What it does",
  inputSchema: { /* parameters */ }
}
```

2. Add handler function:
```javascript
async function myTool(args) {
  // Implement
  return result;
}
```

3. Add case in `CallToolRequestSchema` handler:
```javascript
case "my_tool":
  result = await myTool(args);
  break;
```

4. Update this README with tool documentation

## Integration with oxidizedgraph

The MCP server can test oxidizedgraph orchestration workflows:

```bash
# Start both services
cd /path/to/data-fabric && just dev-worker &
cd /path/to/oxidizedgraph && FABRIC_URL=http://localhost:8787 cargo test &

# In Gemini CLI, test the full orchestration
gemini> Create an orchestration run, ingest governance events, then verify oxidizedgraph \
         nodes can query the data from data-fabric
```

## API Reference

For detailed API documentation, see:
- `docs/LOCAL_DEPLOYMENT.md` — Local setup
- `docs/INTEGRATION_OXIDIZEDGRAPH.md` — oxidizedgraph integration
- `src/lib.rs` — data-fabric API implementation

## Resources

- [Model Context Protocol Spec](https://modelcontextprotocol.io/)
- [Gemini CLI Documentation](https://geminicli.com/)
- [Data Fabric Documentation](../docs/ARCHITECTURE.md)

## License

Apache 2.0 - See LICENSE in parent directory
