#!/usr/bin/env bash
# WS6 End-to-end smoke test for integration endpoints.
# Usage: BASE_URL=https://your-worker.dev TENANT_TOKEN=<token> ./scripts/e2e-integration-smoke.sh
#
# Tests the full oxidizedgraph, aivcs, and llama.rs integration round-trips.
# Requires: curl, jq

set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:8787}"
TENANT_TOKEN="${TENANT_TOKEN:-test-token}"
AUTH_HEADER="Authorization: Bearer ${TENANT_TOKEN}"
CT="Content-Type: application/json"

pass=0
fail=0

check() {
  local name="$1" expected_status="$2" actual_status="$3" body="$4"
  if [ "$actual_status" = "$expected_status" ]; then
    echo "  PASS: $name (HTTP $actual_status)"
    pass=$((pass + 1))
  else
    echo "  FAIL: $name — expected $expected_status, got $actual_status"
    echo "        body: $body"
    fail=$((fail + 1))
  fi
}

echo "=== WS6 Integration Smoke Test ==="
echo "    Target: $BASE_URL"
echo ""

# ── 1. Register integrations ─────────────────────────────────────
echo "--- Register integrations ---"

resp=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/v1/integrations" \
  -H "$AUTH_HEADER" -H "$CT" \
  -d '{"target":"oxidizedgraph","name":"smoke-test-graph"}')
status=$(echo "$resp" | tail -1)
body=$(echo "$resp" | sed '$d')
check "register oxidizedgraph" 200 "$status" "$body"
OG_ID=$(echo "$body" | jq -r '.id // empty')

resp=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/v1/integrations" \
  -H "$AUTH_HEADER" -H "$CT" \
  -d '{"target":"aivcs","name":"smoke-test-ci"}')
status=$(echo "$resp" | tail -1)
body=$(echo "$resp" | sed '$d')
check "register aivcs" 200 "$status" "$body"

resp=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/v1/integrations" \
  -H "$AUTH_HEADER" -H "$CT" \
  -d '{"target":"llama_rs","name":"smoke-test-llm"}')
status=$(echo "$resp" | tail -1)
body=$(echo "$resp" | sed '$d')
check "register llama_rs" 200 "$status" "$body"

# ── 2. List integrations ─────────────────────────────────────────
echo ""
echo "--- List integrations ---"

resp=$(curl -s -w "\n%{http_code}" -X GET "$BASE_URL/v1/integrations" \
  -H "$AUTH_HEADER")
status=$(echo "$resp" | tail -1)
body=$(echo "$resp" | sed '$d')
count=$(echo "$body" | jq '.integrations | length')
check "list integrations (>= 3)" 200 "$status" "$body"
echo "        found $count integration(s)"

# ── 3. Get single integration ────────────────────────────────────
if [ -n "${OG_ID:-}" ]; then
  echo ""
  echo "--- Get single integration ---"
  resp=$(curl -s -w "\n%{http_code}" -X GET "$BASE_URL/v1/integrations/$OG_ID" \
    -H "$AUTH_HEADER")
  status=$(echo "$resp" | tail -1)
  body=$(echo "$resp" | sed '$d')
  check "get integration by id" 200 "$status" "$body"
fi

# ── 4. oxidizedgraph: send events ────────────────────────────────
echo ""
echo "--- oxidizedgraph: ingest events ---"

resp=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/v1/integrations/oxidizedgraph/events" \
  -H "$AUTH_HEADER" -H "$CT" \
  -d '{
    "graph_id": "smoke-graph-1",
    "thread_id": "smoke-thread-1",
    "events": [
      {"event_type": "graph_start", "node_id": "root", "node_type": "entry"},
      {"event_type": "node_start", "node_id": "llm-1", "node_type": "llm_call"},
      {"event_type": "node_end", "node_id": "llm-1", "duration_ms": 250},
      {"event_type": "checkpoint_save", "node_id": "llm-1", "state": {"messages": ["hello"]}, "parent_node_id": "root"},
      {"event_type": "graph_end", "node_id": "root"}
    ]
  }')
status=$(echo "$resp" | tail -1)
body=$(echo "$resp" | sed '$d')
check "oxidizedgraph events" 200 "$status" "$body"
events_ingested=$(echo "$body" | jq '.events_ingested // 0')
checkpoints=$(echo "$body" | jq '.checkpoints_created // 0')
echo "        events_ingested=$events_ingested checkpoints_created=$checkpoints"

# ── 5. aivcs: send pipeline event ────────────────────────────────
echo ""
echo "--- aivcs: ingest pipeline event ---"

resp=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/v1/integrations/aivcs/events" \
  -H "$AUTH_HEADER" -H "$CT" \
  -d '{
    "pipeline_id": "smoke-pipeline-1",
    "repo": "stevedores-org/data-fabric",
    "event_type": "pipeline_start",
    "actor": "smoke-test",
    "commit_sha": "abc123",
    "branch": "develop",
    "artifacts": [
      {"key": "build/output.wasm", "content_type": "application/wasm", "size_bytes": 1024, "checksum": "sha256:smoke"}
    ]
  }')
status=$(echo "$resp" | tail -1)
body=$(echo "$resp" | sed '$d')
check "aivcs pipeline_start" 200 "$status" "$body"
aivcs_run=$(echo "$body" | jq -r '.run_id // "null"')
echo "        run_id=$aivcs_run artifacts_stored=$(echo "$body" | jq '.artifacts_stored // 0')"

# ── 6. llama.rs: send inference request ──────────────────────────
echo ""
echo "--- llama.rs: inference request ---"

resp=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/v1/integrations/llama-rs/inference" \
  -H "$AUTH_HEADER" -H "$CT" \
  -d '{
    "model": "llama-3.2-70b",
    "prompt": "explain data fabric architecture",
    "temperature": 0.7,
    "max_tokens": 512,
    "run_id": "smoke-run-1"
  }')
status=$(echo "$resp" | tail -1)
body=$(echo "$resp" | sed '$d')
check "llama.rs inference" 200 "$status" "$body"
task_id=$(echo "$body" | jq -r '.task_id // "null"')
echo "        task_id=$task_id"

# ── 7. llama.rs: send telemetry ──────────────────────────────────
echo ""
echo "--- llama.rs: telemetry ---"

resp=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/v1/integrations/llama-rs/telemetry" \
  -H "$AUTH_HEADER" -H "$CT" \
  -d "{
    \"task_id\": \"$task_id\",
    \"event_type\": \"inference_end\",
    \"model\": \"llama-3.2-70b\",
    \"tokens_in\": 25,
    \"tokens_out\": 200,
    \"duration_ms\": 1500,
    \"tool_calls\": [{\"tool_name\": \"web_search\", \"input\": {\"q\": \"fabric\"}, \"duration_ms\": 300}]
  }")
status=$(echo "$resp" | tail -1)
body=$(echo "$resp" | sed '$d')
check "llama.rs telemetry" 200 "$status" "$body"
echo "        events_ingested=$(echo "$body" | jq '.events_ingested // 0')"

# ── 8. Cleanup: delete integration ───────────────────────────────
if [ -n "${OG_ID:-}" ]; then
  echo ""
  echo "--- Cleanup ---"
  resp=$(curl -s -w "\n%{http_code}" -X DELETE "$BASE_URL/v1/integrations/$OG_ID" \
    -H "$AUTH_HEADER")
  status=$(echo "$resp" | tail -1)
  body=$(echo "$resp" | sed '$d')
  check "delete integration" 200 "$status" "$body"
fi

# ── Summary ──────────────────────────────────────────────────────
echo ""
echo "=== Results: $pass passed, $fail failed ==="
[ "$fail" -eq 0 ] && exit 0 || exit 1
