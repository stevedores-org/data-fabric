# Data Fabric + Gemini CLI Integration Guide

**Test data-fabric using Gemini AI as your testing agent.**

## Quick Start (3 steps)

### 1. Start data-fabric

```bash
cd /path/to/data-fabric
just dev-worker
```

### 2. Register MCP server with Gemini CLI

```bash
cd /path/to/data-fabric/mcp-server

# Install dependencies
npm install

# Register extension
gemini extensions add .

# Verify
gemini extensions list
```

### 3. Start testing with Gemini

```bash
gemini

# Now ask Gemini to test data-fabric:
> Test if data-fabric is healthy
```

## Example Testing Sessions

### Session 1: Health Check & Report

```bash
$ gemini

> Check if data-fabric is running and healthy, then tell me the status

Gemini will:
1. Call health_check tool
2. Parse response
3. Report: "✅ data-fabric is running and healthy"
```

### Session 2: Full Workflow Test

```bash
$ gemini

> Create a new run, ingest 3 events (started, processed, completed), \
> then query the context and verify all events were recorded

Gemini will:
1. create_run(spec_sha="test...", flow="test_workflow")
2. ingest_event(run_id=<from step 1>, event_type="tool_called", payload={...})
3. ingest_event(...) — 2 more times
4. query_context(query_type="memory", run_id=<from step 1>)
5. Verify and summarize results
```

### Session 3: Governance Workflow Test

```bash
$ gemini

> Test a governance workflow:
> 1. Create a run with spec for governance node
> 2. Ingest governance-related events (guidance_applied, rules_checked, etc)
> 3. Query lineage to verify the governance decision chain
> 4. Report any compliance issues

Gemini will fully orchestrate the test
```

### Session 4: Multi-Tenant Setup Test

```bash
$ gemini

> Set up a test environment:
> 1. Provision a new tenant "acme-corp"
> 2. Create 3 runs in the new tenant
> 3. Show me all runs for that tenant
> 4. Compare with my current tenant

Gemini will handle all tenant management
```

## What Gemini Can Do

### Automated Testing

```bash
> Run a complete integration test: provision tenant → create run → execute workflow → verify
```

### Issue Detection & Diagnosis

```bash
> Check data-fabric health and if anything looks wrong, diagnose the issue
```

### Performance Analysis

```bash
> Create 10 runs with events and measure how long queries take, suggest optimizations
```

### Compliance Validation

```bash
> Test that governance rules are properly enforced by creating conflicting events and verifying they're blocked
```

### Report Generation

```bash
> Run a comprehensive test suite and generate a markdown report with results, timing, and recommendations
```

## Advanced: Using Gemini API Directly

For programmatic testing, you can call Gemini API directly:

```bash
# Set API key
export GOOGLE_API_KEY="your-gemini-api-key"

# Use via node
node -e "
const Anthropic = require('@google/generative-ai');
const client = new Anthropic.GoogleGenerativeAI(process.env.GOOGLE_API_KEY);

async function test() {
  const model = client.getGenerativeModel({
    model: 'gemini-2.0-flash',
    tools: [{
      functionDeclarations: [
        {
          name: 'health_check',
          description: 'Check data-fabric health'
        }
      ]
    }]
  });

  const response = await model.generateContent('Check if data-fabric is healthy');
  console.log(response);
}

test();
"
```

## Integration with oxidizedgraph Testing

Test the full orchestration stack:

```bash
# Terminal 1: Start data-fabric
cd /path/to/data-fabric && just dev-worker

# Terminal 2: Start oxidizedgraph test server
cd /path/to/oxidizedgraph && cargo run --example orchestration_test

# Terminal 3: Use Gemini to orchestrate testing
gemini

> Execute a governance workflow through oxidizedgraph that:
> 1. Creates a run in data-fabric
> 2. Executes a governance node in oxidizedgraph
> 3. Records the output in data-fabric
> 4. Verifies compliance rules were applied
```

## Configuration

### Environment Variables

```bash
# Point to different data-fabric instance
export FABRIC_URL="http://staging.example.com:8787"

# Use specific tenant
export TENANT_ID="my-org"
export TENANT_ROLE="admin"

# Gemini API configuration
export GOOGLE_API_KEY="your-api-key-here"

# Start Gemini CLI
gemini
```

### Persistent Configuration

Create `.gemini-config.json`:

```json
{
  "extensions": {
    "data-fabric-mcp": {
      "enabled": true,
      "environment": {
        "FABRIC_URL": "http://localhost:8787",
        "TENANT_ID": "test-tenant"
      }
    }
  },
  "defaultModel": "gemini-2.0-flash",
  "timeout": 30000
}
```

## Testing Patterns

### Pattern 1: Chaos Testing

```bash
> Create a run, then ingest conflicting events in random order and verify data-fabric handles them correctly
```

### Pattern 2: Load Testing

```bash
> Create 100 runs and ingest 1000 events across them, measure latency and identify bottlenecks
```

### Pattern 3: Compliance Testing

```bash
> Verify that governance rules in data-fabric prevent unauthorized actions by attempting to violate them
```

### Pattern 4: Integration Testing

```bash
> Execute a full orchestration workflow: data-fabric + oxidizedgraph + governance + llm nodes, verify all components work together
```

## Troubleshooting

### "Extension not loaded"

```bash
# Re-register
gemini extensions add /path/to/data-fabric/mcp-server

# Restart Gemini CLI
gemini
```

### "Tool not available"

```bash
# Verify MCP server is running
ps aux | grep data-fabric-mcp

# If not, start it
cd /path/to/data-fabric/mcp-server
npm start
```

### "API calls failing"

```bash
# Check data-fabric health
curl http://localhost:8787/health

# Check MCP server logs
tail -f /tmp/data-fabric-mcp.log

# Verify environment variables
echo $FABRIC_URL
echo $TENANT_ID
```

## Next Steps

1. **Start data-fabric**: `just dev-worker`
2. **Register MCP server**: `gemini extensions add ./mcp-server`
3. **Open Gemini CLI**: `gemini`
4. **Ask it to test**: `"Test data-fabric health and create a run"`

---

**Real-world testing with AI! The ultimate integration testing experience.** 🤖
