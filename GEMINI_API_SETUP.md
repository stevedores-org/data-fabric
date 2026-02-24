# Gemini API Setup for Data Fabric Testing

Enable AI-powered testing of data-fabric using Google's Gemini API.

## Prerequisites

- Gemini CLI installed: `npm install -g @google/gemini-cli`
- Google account with access to Gemini API
- data-fabric running locally: `just dev-worker`

## Step 1: Get Gemini API Key

### Option A: Web UI (Easy)

1. Go to [Google AI Studio](https://aistudio.google.com/apikey)
2. Click "Get API Key"
3. Create a new API key or use existing one
4. Copy the key

### Option B: Google Cloud Console

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project or select existing
3. Enable Generative AI API
4. Go to Credentials → Create API Key
5. Copy the key

## Step 2: Store API Key (Recommended for macOS)

### macOS - Use Keychain (Secure)

```bash
# Store API key in Keychain
security add-generic-password -s "GEMINI_API_KEY" -a "gemini" -w "your-api-key-here"

# Verify it's stored
security find-generic-password -s "GEMINI_API_KEY"

# Export in your shell session
export GEMINI_API_KEY=$(security find-generic-password -s "GEMINI_API_KEY" -w)

# Add to ~/.zshrc for permanent setup
echo 'export GEMINI_API_KEY=$(security find-generic-password -s "GEMINI_API_KEY" -w)' >> ~/.zshrc
source ~/.zshrc
```

### Linux - Use Environment Variable

```bash
# Store in .env file
cp .env.example .env
# Edit .env and set GOOGLE_API_KEY=your-key

# Export in shell
export GOOGLE_API_KEY="your-api-key-here"

# Make permanent in ~/.bashrc or ~/.zshrc
echo 'export GOOGLE_API_KEY="your-api-key-here"' >> ~/.bashrc
source ~/.bashrc
```

### Windows (PowerShell)

```powershell
# Set environment variable
$env:GOOGLE_API_KEY = "your-api-key-here"

# Make it permanent
[System.Environment]::SetEnvironmentVariable("GOOGLE_API_KEY", "your-api-key-here", "User")

# Verify
$env:GOOGLE_API_KEY
```

## Step 3: Configure Gemini CLI

### Initialize Gemini CLI

```bash
gemini init

# Follow prompts:
# - Model: Select "gemini-2.0-flash" (latest, fastest)
# - Extensions: Skip for now
# - API Key: Will auto-detect from GOOGLE_API_KEY or GEMINI_API_KEY env var
```

### Verify Configuration

```bash
gemini --version
# Output: gemini version X.X.X

gemini --help
# Should show command options
```

## Step 4: Set Up Data Fabric MCP Extension

### Install MCP Server

```bash
cd /path/to/data-fabric/mcp-server

# Install dependencies
npm install

# Register with Gemini CLI
gemini extensions add .

# Verify
gemini extensions list
# Should show: data-fabric-mcp
```

### Configure for Local Testing

```bash
# Set environment variables
export FABRIC_URL=http://localhost:8787
export TENANT_ID=test-tenant
export TENANT_ROLE=admin

# Or create .env file in mcp-server/
cp ../.env.example mcp-server/.env
# Edit mcp-server/.env with your values
```

## Step 5: Test the Integration

### Start Data Fabric

```bash
# Terminal 1
cd /path/to/data-fabric
just dev-worker
# ✨ Listening on http://localhost:8787
```

### Test with Gemini CLI

```bash
# Terminal 2
export GOOGLE_API_KEY=$(security find-generic-password -s "GEMINI_API_KEY" -w)  # macOS

gemini

# In the Gemini prompt, test data-fabric:
> Check if data-fabric is healthy

# Output:
# Gemini will call the health_check tool and report:
# ✅ data-fabric is running and healthy
# Status: ok
# Service: data-fabric
# Mission: velocity-for-autonomous-agent-builders
```

## Testing Workflows

### Test 1: Basic Health Check

```bash
gemini> Check if data-fabric is healthy and report any issues
```

### Test 2: Full Workflow

```bash
gemini> Create a new run, ingest 3 test events (started, processed, completed), \
         then query the context to verify all events were recorded
```

### Test 3: Governance Testing

```bash
gemini> Test a governance workflow:
         1. Create a run with spec_sha="governance-test"
         2. Ingest governance-related events
         3. Query lineage to verify the decision chain
         4. Report compliance status
```

### Test 4: Integration Test

```bash
gemini> Test oxidizedgraph + data-fabric integration:
         1. Create run in data-fabric
         2. Simulate orchestration workflow
         3. Ingest events
         4. Query and verify all components worked together
```

## Advanced: Use Gemini API Directly

For programmatic testing without CLI:

```javascript
// test-with-gemini-api.js
const { GoogleGenerativeAI } = require("@google/generative-ai");

const genAI = new GoogleGenerativeAI(process.env.GOOGLE_API_KEY);

async function testDataFabric() {
  const model = genAI.getGenerativeModel({
    model: "gemini-2.0-flash",
    tools: [{
      functionDeclarations: [
        {
          name: "health_check",
          description: "Check data-fabric health"
        }
      ]
    }]
  });

  const response = await model.generateContent(
    "Check if data-fabric is healthy and report the status"
  );

  console.log(response.response.text());
}

testDataFabric().catch(console.error);
```

Run with:
```bash
GOOGLE_API_KEY=$(security find-generic-password -s "GEMINI_API_KEY" -w) node test-with-gemini-api.js
```

## Troubleshooting

### "API Key not found"

```bash
# Check environment variable is set
echo $GOOGLE_API_KEY
echo $GEMINI_API_KEY

# If empty, set it
export GOOGLE_API_KEY="your-key-here"

# For macOS Keychain
security find-generic-password -s "GEMINI_API_KEY"
```

### "Tool not available in Gemini"

```bash
# Verify MCP server is registered
gemini extensions list
# Should show: data-fabric-mcp

# Re-register if needed
gemini extensions add /path/to/data-fabric/mcp-server

# Restart Gemini CLI
exit
gemini
```

### "Cannot connect to data-fabric"

```bash
# Check data-fabric is running
curl http://localhost:8787/health
# Should return {"service":"data-fabric",...}

# Check FABRIC_URL is set correctly
echo $FABRIC_URL
# Should be http://localhost:8787
```

### "Rate limited"

Gemini API has rate limits. If you hit them:
- Wait a few minutes before retrying
- Consider batching requests
- Check your API usage at [Google AI Studio](https://aistudio.google.com/app/apikey)

## Security Notes

### ✅ Do This
- Store API key in Keychain (macOS) or secure environment
- Use `.env.example` as template, never commit `.env`
- Rotate API keys periodically
- Use separate keys for different environments
- Restrict API key in Google Cloud Console

### ❌ Don't Do This
- Commit API key to git
- Share API key in chat or logs
- Use same key for dev, staging, production
- Print API key in console output
- Store in plain text files without `.env` in `.gitignore`

## Next Steps

1. **Get API Key**: https://aistudio.google.com/apikey
2. **Store Securely**: Use Keychain (macOS) or environment variable
3. **Initialize Gemini**: `gemini init`
4. **Register MCP Server**: `gemini extensions add ./mcp-server`
5. **Test**: `gemini` → "Check if data-fabric is healthy"

## Resources

- [Google AI Studio](https://aistudio.google.com/)
- [Gemini API Documentation](https://ai.google.dev/docs)
- [Gemini CLI Documentation](https://geminicli.com/docs/)
- [Model Context Protocol](https://modelcontextprotocol.io/)

---

**Your data-fabric is now ready for AI-powered testing!** 🤖
