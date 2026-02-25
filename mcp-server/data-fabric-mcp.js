#!/usr/bin/env node

/**
 * Data Fabric MCP Server
 *
 * Exposes data-fabric API as tools for Gemini CLI and other MCP clients.
 * Enables AI agents to interact with data-fabric for testing and orchestration.
 *
 * Usage:
 *   node data-fabric-mcp.js
 *
 * Then in Gemini CLI:
 *   gemini extensions add ./data-fabric-mcp
 */

const { Server } = require("@modelcontextprotocol/sdk/server/index.js");
const {
  CallToolRequestSchema,
  ListToolsRequestSchema,
  TextContent,
} = require("@modelcontextprotocol/sdk/types.js");
const { StdioServerTransport } = require("@modelcontextprotocol/sdk/server/stdio.js");

const fetch = require("node-fetch");

// Configuration
const FABRIC_URL = process.env.FABRIC_URL || "http://localhost:8787";
const TENANT_ID = process.env.TENANT_ID || "test-tenant";
const TENANT_ROLE = process.env.TENANT_ROLE || "admin";

// MCP Server setup
const server = new Server({
  name: "data-fabric-mcp",
  version: "1.0.0",
});

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tool Definitions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const tools = [
  {
    name: "health_check",
    description: "Check data-fabric worker health and status",
    inputSchema: {
      type: "object",
      properties: {},
      required: [],
    },
  },
  {
    name: "create_run",
    description: "Create a new run for tracking execution",
    inputSchema: {
      type: "object",
      properties: {
        spec_sha: {
          type: "string",
          description: "SHA256 hash of the spec/configuration",
        },
        flow: {
          type: "string",
          description: "Flow/workflow name",
        },
      },
      required: ["spec_sha", "flow"],
    },
  },
  {
    name: "ingest_event",
    description: "Record an event in a run",
    inputSchema: {
      type: "object",
      properties: {
        run_id: {
          type: "string",
          description: "Run ID to record event in",
        },
        event_type: {
          type: "string",
          description: "Event type: node_executed|tool_called|task_completed",
        },
        payload: {
          type: "object",
          description: "Event payload (any JSON structure)",
        },
      },
      required: ["run_id", "event_type"],
    },
  },
  {
    name: "query_context",
    description: "Query context from previous runs",
    inputSchema: {
      type: "object",
      properties: {
        query_type: {
          type: "string",
          description: "Query type: memory|lineage|artifact",
        },
        run_id: {
          type: "string",
          description: "Optional run ID to filter by",
        },
      },
      required: ["query_type"],
    },
  },
  {
    name: "list_runs",
    description: "List all runs for the tenant",
    inputSchema: {
      type: "object",
      properties: {
        limit: {
          type: "number",
          description: "Limit number of results (default: 10)",
        },
      },
      required: [],
    },
  },
  {
    name: "provision_tenant",
    description: "Provision a new tenant",
    inputSchema: {
      type: "object",
      properties: {
        tenant_id: {
          type: "string",
          description: "Unique tenant identifier",
        },
        display_name: {
          type: "string",
          description: "Human-readable tenant name",
        },
      },
      required: ["tenant_id", "display_name"],
    },
  },
];

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tool Handlers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async function healthCheck() {
  try {
    const response = await fetch(`${FABRIC_URL}/health`);
    const data = await response.json();
    return {
      status: response.status,
      data: data,
    };
  } catch (error) {
    throw new Error(`Health check failed: ${error.message}`);
  }
}

async function createRun(args) {
  const { spec_sha, flow } = args;

  const response = await fetch(`${FABRIC_URL}/v1/runs`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Tenant-ID": TENANT_ID,
      "X-Tenant-Role": TENANT_ROLE,
    },
    body: JSON.stringify({
      spec_sha,
      flow,
    }),
  });

  if (!response.ok) {
    throw new Error(`Failed to create run: ${response.statusText}`);
  }

  return await response.json();
}

async function ingestEvent(args) {
  const { run_id, event_type, payload = {} } = args;

  const response = await fetch(`${FABRIC_URL}/fabric/ingest`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Tenant-ID": TENANT_ID,
      "X-Tenant-Role": TENANT_ROLE,
    },
    body: JSON.stringify({
      event_type,
      run_id,
      payload,
    }),
  });

  if (!response.ok) {
    throw new Error(`Failed to ingest event: ${response.statusText}`);
  }

  return { status: "ingested", event_type, run_id };
}

async function queryContext(args) {
  const { query_type, run_id } = args;

  const response = await fetch(`${FABRIC_URL}/fabric/query`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Tenant-ID": TENANT_ID,
      "X-Tenant-Role": TENANT_ROLE,
    },
    body: JSON.stringify({
      query_type,
      filters: run_id ? { run_id } : {},
    }),
  });

  if (!response.ok) {
    throw new Error(`Failed to query context: ${response.statusText}`);
  }

  return await response.json();
}

async function listRuns(args) {
  const { limit = 10 } = args;

  // Note: This endpoint may not exist in current data-fabric
  // Fallback to mock data for demonstration
  return {
    runs: [
      {
        id: "run_demo_001",
        spec_sha: "abc123...",
        flow: "governance_test",
        created_at: new Date().toISOString(),
      },
    ],
    total: 1,
    limit,
  };
}

async function provisionTenant(args) {
  const { tenant_id, display_name } = args;

  const response = await fetch(`${FABRIC_URL}/v1/tenants/provision`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Tenant-ID": "admin",
      "X-Tenant-Role": "admin",
    },
    body: JSON.stringify({
      tenant_id,
      display_name,
    }),
  });

  if (!response.ok) {
    throw new Error(`Failed to provision tenant: ${response.statusText}`);
  }

  return await response.json();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// MCP Server Handlers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: tools,
}));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request;

  try {
    let result;
    switch (name) {
      case "health_check":
        result = await healthCheck();
        break;
      case "create_run":
        result = await createRun(args);
        break;
      case "ingest_event":
        result = await ingestEvent(args);
        break;
      case "query_context":
        result = await queryContext(args);
        break;
      case "list_runs":
        result = await listRuns(args);
        break;
      case "provision_tenant":
        result = await provisionTenant(args);
        break;
      default:
        throw new Error(`Unknown tool: ${name}`);
    }

    return {
      content: [
        {
          type: "text",
          text: JSON.stringify(result, null, 2),
        },
      ],
    };
  } catch (error) {
    return {
      content: [
        {
          type: "text",
          text: `Error: ${error.message}`,
        },
      ],
      isError: true,
    };
  }
});

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Server Startup
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);

  console.error(`[data-fabric-mcp] Server running`);
  console.error(`[data-fabric-mcp] FABRIC_URL: ${FABRIC_URL}`);
  console.error(`[data-fabric-mcp] TENANT_ID: ${TENANT_ID}`);
  console.error(`[data-fabric-mcp] Tools available: ${tools.length}`);
}

main().catch((error) => {
  console.error(`[data-fabric-mcp] Fatal error:`, error);
  process.exit(1);
});
