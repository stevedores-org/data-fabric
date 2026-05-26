#!/usr/bin/env bash
# WS10 pilot endpoint smoke test.
#
# Hits GET /v1/metrics/pilot and asserts the response has the expected
# top-level shape. Intended for reviewer / deployer sanity checks — NOT
# a substitute for unit tests.
#
# Usage:
#   scripts/pilot-smoke.sh                            # local dev (8787)
#   scripts/pilot-smoke.sh --base-url https://data-fabric.stevedores.org
#   scripts/pilot-smoke.sh --tenant-id acme --window 7d
#   scripts/pilot-smoke.sh --task-type oxidizedgraph.test
#
# Exit codes:
#   0   response is 200 and shape is valid
#   2   missing prereqs (curl, jq)
#   3   request failed (network or non-200)
#   4   response is 200 but shape is invalid (missing top-level keys)
set -euo pipefail

BASE_URL="http://127.0.0.1:8787"
TENANT_ID="smoke-tenant"
WINDOW="1d"
TASK_TYPE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base-url)  BASE_URL="$2";  shift 2 ;;
    --tenant-id) TENANT_ID="$2"; shift 2 ;;
    --window)    WINDOW="$2";    shift 2 ;;
    --task-type) TASK_TYPE="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,15p' "$0"
      exit 0
      ;;
    *) echo "unknown arg: $1" >&2; exit 64 ;;
  esac
done

# ── Prereqs ─────────────────────────────────────────────────────
for bin in curl jq; do
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "error: required binary not found: $bin" >&2
    exit 2
  fi
done

URL="${BASE_URL}/v1/metrics/pilot?window=${WINDOW}"
if [[ -n "$TASK_TYPE" ]]; then
  # URL-encode so values like "foo bar" or "ns:type" don't break the query.
  TASK_TYPE_ENC=$(jq -rn --arg v "$TASK_TYPE" '$v|@uri')
  URL="${URL}&task_type=${TASK_TYPE_ENC}"
fi

echo "→ GET ${URL}"
echo "  x-tenant-id: ${TENANT_ID}"

# Capture body and status separately so a non-200 still gives a body to inspect.
BODY_FILE=$(mktemp)
trap 'rm -f "$BODY_FILE"' EXIT
HTTP_STATUS=$(curl -sS -o "$BODY_FILE" -w "%{http_code}" \
  -H "x-tenant-id: ${TENANT_ID}" \
  "$URL") || {
    echo "error: curl failed" >&2
    exit 3
}

if [[ "$HTTP_STATUS" != "200" ]]; then
  echo "error: expected 200, got ${HTTP_STATUS}" >&2
  cat "$BODY_FILE" >&2
  exit 3
fi

# ── Shape assertions ────────────────────────────────────────────
REQUIRED_KEYS=(window window_seconds sample_counts kpis meta)
MISSING=()
for key in "${REQUIRED_KEYS[@]}"; do
  if ! jq -e --arg k "$key" 'has($k)' "$BODY_FILE" >/dev/null; then
    MISSING+=("$key")
  fi
done

if (( ${#MISSING[@]} > 0 )); then
  echo "error: response missing top-level keys: ${MISSING[*]}" >&2
  jq '.' "$BODY_FILE" >&2
  exit 4
fi

REQUIRED_KPIS=(
  task_completion_rate
  mttr_p50_seconds
  mttr_p95_seconds
  context_reuse_rate
  human_intervention_rate
  event_throughput_per_sec
)
MISSING_KPIS=()
for k in "${REQUIRED_KPIS[@]}"; do
  if ! jq -e --arg k "$k" '.kpis | has($k)' "$BODY_FILE" >/dev/null; then
    MISSING_KPIS+=("$k")
  fi
done

if (( ${#MISSING_KPIS[@]} > 0 )); then
  echo "error: kpis missing: ${MISSING_KPIS[*]}" >&2
  jq '.kpis' "$BODY_FILE" >&2
  exit 4
fi

echo "✓ HTTP 200 with valid shape"
jq '.' "$BODY_FILE"
