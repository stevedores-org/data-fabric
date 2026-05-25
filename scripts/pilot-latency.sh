#!/usr/bin/env bash
# WS10 pilot KPI #6: API latency p50/p95/p99 from Workers Analytics Engine.
#
# Pairs with GET /v1/metrics/pilot — that endpoint covers the 5 D1-backed
# KPIs; this script covers the latency one because Analytics Engine queries
# go through Cloudflare's SQL API, not D1.
#
# Usage:
#   scripts/pilot-latency.sh                  # default window: 1d
#   scripts/pilot-latency.sh --window 1h
#   scripts/pilot-latency.sh --window 7d --dataset data_fabric_pilot_latency
#
# Requires:
#   CLOUDFLARE_API_TOKEN   API token with `Account Analytics: Read`
#   (account_id is read from wrangler.toml; override via CF_ACCOUNT_ID)
#
# Exit codes:
#   0   query succeeded (may report "no data" if dataset is empty)
#   2   missing prereqs (token, jq, curl)
#   3   API request failed
#   4   dataset has no rows in window — Worker may not be emitting yet
set -euo pipefail

WINDOW="1d"
DATASET="data_fabric_pilot_latency"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --window)  WINDOW="$2"; shift 2 ;;
    --dataset) DATASET="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,20p' "$0"
      exit 0
      ;;
    *) echo "unknown arg: $1" >&2; exit 64 ;;
  esac
done

# ── Resolve window → INTERVAL clause ────────────────────────────
case "$WINDOW" in
  *h) INTERVAL="INTERVAL '${WINDOW%h}' HOUR" ;;
  *d) INTERVAL="INTERVAL '${WINDOW%d}' DAY" ;;
  *w) INTERVAL="INTERVAL '$(( ${WINDOW%w} * 7 ))' DAY" ;;
  *)  echo "bad --window: $WINDOW (expected Nh/Nd/Nw)" >&2; exit 64 ;;
esac

# ── Prereq checks ───────────────────────────────────────────────
if [[ -z "${CLOUDFLARE_API_TOKEN:-}" ]]; then
  echo "error: CLOUDFLARE_API_TOKEN not set" >&2
  echo "  scope needed: Account Analytics: Read" >&2
  exit 2
fi
for bin in curl jq; do
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "error: required binary not found: $bin" >&2
    exit 2
  fi
done

# ── Resolve account_id ──────────────────────────────────────────
ACCOUNT_ID="${CF_ACCOUNT_ID:-}"
if [[ -z "$ACCOUNT_ID" ]]; then
  ACCOUNT_ID=$(awk -F'"' '/^account_id/ {print $2; exit}' \
                 "$(dirname "$0")/../wrangler.toml")
fi
if [[ -z "$ACCOUNT_ID" ]]; then
  echo "error: could not resolve account_id (set CF_ACCOUNT_ID)" >&2
  exit 2
fi

# ── Build SQL and POST to the Analytics Engine SQL API ──────────
# Schema assumption: writeDataPoint emits latency_ms as double1.
# When the Worker starts emitting to this dataset, this schema can be
# tightened — see docs/ws10/METRICS_ENDPOINT.md.
SQL="SELECT
  quantileWeighted(0.50)(double1, _sample_interval) AS p50_ms,
  quantileWeighted(0.95)(double1, _sample_interval) AS p95_ms,
  quantileWeighted(0.99)(double1, _sample_interval) AS p99_ms,
  count() AS sample_count
FROM ${DATASET}
WHERE timestamp >= NOW() - ${INTERVAL}"

API="https://api.cloudflare.com/client/v4/accounts/${ACCOUNT_ID}/analytics_engine/sql"

RESPONSE=$(curl -sS \
  -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" \
  -H "Content-Type: text/plain" \
  --data "$SQL" \
  "$API")

# ── Parse + report ──────────────────────────────────────────────
if ! echo "$RESPONSE" | jq -e '.data' >/dev/null 2>&1; then
  echo "error: Analytics Engine query failed" >&2
  echo "$RESPONSE" >&2
  exit 3
fi

SAMPLE_COUNT=$(echo "$RESPONSE" | jq -r '.data[0].sample_count // 0')
if [[ "$SAMPLE_COUNT" == "0" ]]; then
  echo "no rows in dataset '${DATASET}' over window ${WINDOW}"
  echo "(Worker emits one row per non-public request via emit_pilot_latency"
  echo " in src/lib.rs — if this is unexpectedly empty, confirm the"
  echo " PILOT_LATENCY binding is wired in wrangler.toml for this env and"
  echo " that the deployed build is recent.)"
  exit 4
fi

echo "$RESPONSE" | jq --arg w "$WINDOW" --arg ds "$DATASET" '{
  window:        $w,
  dataset:       $ds,
  sample_count:  .data[0].sample_count,
  p50_ms:        .data[0].p50_ms,
  p95_ms:        .data[0].p95_ms,
  p99_ms:        .data[0].p99_ms
}'
