#!/usr/bin/env bash
# scripts/pilot-latency.sh
#
# WS10 Pilot baseline: fetch p50/p95/p99 API request latency for the
# data-fabric Worker over the same window served by GET /v1/metrics/pilot.
#
# This is the deployment-side companion to the in-fabric KPIs: the Rust
# endpoint surfaces tenant-scoped business KPIs from D1, while this script
# pulls the platform-level latency distribution from Cloudflare Workers
# Analytics (which is not tenant-aware).
#
# Usage:
#   scripts/pilot-latency.sh [WINDOW]
#
#   WINDOW (default: 1d) is the same shape as the metrics endpoint accepts:
#     30m  120s  2h  1d  7d   (must be in [60s, 30d])
#
# Environment:
#   WORKER_NAME     Worker name (default: data-fabric-worker)
#   WRANGLER        wrangler invocation override (default: bunx wrangler@3)
#
# Examples:
#   scripts/pilot-latency.sh
#   scripts/pilot-latency.sh 7d
#   WORKER_NAME=data-fabric-staging scripts/pilot-latency.sh 2h
#
# Notes:
#   - `wrangler analytics` is wrangler@3's analytics CLI; the schema may vary
#     across Cloudflare API changes. The script tolerates that by passing
#     `--json` and letting `jq` extract percentiles.
#   - If you do not have `bunx`/`wrangler`/`jq` installed locally, run from a
#     shell that does (or set WRANGLER='npx wrangler@3').

set -euo pipefail

WINDOW="${1:-1d}"
WORKER_NAME="${WORKER_NAME:-data-fabric-worker}"
WRANGLER="${WRANGLER:-bunx wrangler@3}"

err() { printf '%s\n' "$*" >&2; }

# ── Parse the window into seconds (mirrors src/metrics.rs::parse_window) ──
parse_window_seconds() {
    local raw="$1"
    if [[ -z "$raw" ]]; then
        err "ERROR: window is empty"; return 1
    fi
    local n="${raw%?}"
    local unit="${raw: -1}"
    if ! [[ "$n" =~ ^[0-9]+$ ]] || [[ "$n" -le 0 ]]; then
        err "ERROR: window '$raw' must look like '1d', '2h', '30m', '120s'"
        return 1
    fi
    local secs
    case "$unit" in
        s) secs="$n" ;;
        m) secs="$(( n * 60 ))" ;;
        h) secs="$(( n * 3600 ))" ;;
        d) secs="$(( n * 86400 ))" ;;
        *) err "ERROR: window unit '$unit' not in [s,m,h,d]"; return 1 ;;
    esac
    if [[ "$secs" -lt 60 ]]; then
        err "ERROR: window '$raw' below 60s floor"; return 1
    fi
    if [[ "$secs" -gt 2592000 ]]; then
        err "ERROR: window '$raw' above 30d ceiling"; return 1
    fi
    printf '%s\n' "$secs"
}

WINDOW_SECONDS="$(parse_window_seconds "$WINDOW")"

# ── Tool sanity checks ──────────────────────────────────────────────
if ! command -v jq >/dev/null 2>&1; then
    err "ERROR: 'jq' is required but not on PATH"
    exit 2
fi

# ── Compute the since timestamp in RFC3339 (UTC) ───────────────────
# Cloudflare analytics queries accept ISO-8601 / RFC3339 timestamps.
# Use `date -u -v` (BSD/macOS) or `date -u -d` (GNU) — try both.
since_ts=""
until_ts="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
if since_ts="$(date -u -v"-${WINDOW_SECONDS}S" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null)"; then
    : # BSD date worked
elif since_ts="$(date -u -d "@$(( $(date +%s) - WINDOW_SECONDS ))" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null)"; then
    : # GNU date worked
else
    err "ERROR: could not compute since timestamp (neither BSD nor GNU date worked)"
    exit 3
fi

err "[pilot-latency] worker=${WORKER_NAME} window=${WINDOW} (${WINDOW_SECONDS}s)"
err "[pilot-latency] since=${since_ts} until=${until_ts}"

# ── Invoke wrangler analytics ──────────────────────────────────────
# Wrangler's `analytics` subcommand surface has evolved; we try a couple
# of shapes and fall through to the most permissive one. The point of
# this script is to gather a single number per percentile.
tmp_out="$(mktemp -t pilot-latency.XXXXXX.json)"
trap 'rm -f "$tmp_out"' EXIT

# Attempt 1: wrangler 3+ stable shape.
if ! ${WRANGLER} analytics \
        --name "${WORKER_NAME}" \
        --since "${since_ts}" \
        --until "${until_ts}" \
        --json \
        > "$tmp_out" 2>/dev/null
then
    err "[pilot-latency] primary 'wrangler analytics' invocation failed"
    err "[pilot-latency] retrying with the bare 'analytics' shape"
    # Attempt 2: some wrangler versions take the worker name positionally.
    if ! ${WRANGLER} analytics "${WORKER_NAME}" \
            --since "${since_ts}" \
            --until "${until_ts}" \
            --json \
            > "$tmp_out" 2>/dev/null
    then
        err "ERROR: 'wrangler analytics' is unavailable or its CLI shape has changed."
        err "       Inspect:  ${WRANGLER} analytics --help"
        err "       Then update this script to match. Output so far:"
        cat "$tmp_out" >&2 || true
        exit 4
    fi
fi

# ── Extract percentiles via jq ─────────────────────────────────────
# Try the most common analytics field paths Cloudflare exposes.
# We emit a stable JSON envelope so downstream pilot tooling can diff.
out_json="$(jq -c '
  def first_non_null(arr): arr | map(select(. != null)) | .[0];
  {
    worker: env.WORKER_NAME // "",
    window: env.WINDOW // "",
    window_seconds: (env.WINDOW_SECONDS|tonumber? // null),
    since: env.SINCE_TS,
    until: env.UNTIL_TS,
    api_latency_p50_ms: first_non_null([
      .latency.p50, .p50, .request_latency.p50, .latency_ms.p50
    ]),
    api_latency_p95_ms: first_non_null([
      .latency.p95, .p95, .request_latency.p95, .latency_ms.p95
    ]),
    api_latency_p99_ms: first_non_null([
      .latency.p99, .p99, .request_latency.p99, .latency_ms.p99
    ]),
    raw: .
  }
' --arg dummy "" \
  --argjson _unused 0 \
  "$tmp_out" \
  WORKER_NAME="${WORKER_NAME}" \
  WINDOW="${WINDOW}" \
  WINDOW_SECONDS="${WINDOW_SECONDS}" \
  SINCE_TS="${since_ts}" \
  UNTIL_TS="${until_ts}" \
  2>/dev/null || true)"

# Fallback if jq's --arg substitution differs by version — re-emit using env.
if [[ -z "$out_json" ]]; then
    WORKER_NAME="$WORKER_NAME" \
    WINDOW="$WINDOW" \
    WINDOW_SECONDS="$WINDOW_SECONDS" \
    SINCE_TS="$since_ts" \
    UNTIL_TS="$until_ts" \
    out_json="$(jq -c '
      def first_non_null(arr): arr | map(select(. != null)) | .[0];
      {
        worker:           env.WORKER_NAME,
        window:           env.WINDOW,
        window_seconds:   (env.WINDOW_SECONDS|tonumber),
        since:            env.SINCE_TS,
        until:            env.UNTIL_TS,
        api_latency_p50_ms: first_non_null([.latency.p50, .p50, .request_latency.p50, .latency_ms.p50]),
        api_latency_p95_ms: first_non_null([.latency.p95, .p95, .request_latency.p95, .latency_ms.p95]),
        api_latency_p99_ms: first_non_null([.latency.p99, .p99, .request_latency.p99, .latency_ms.p99]),
        raw: .
      }
    ' "$tmp_out")"
fi

printf '%s\n' "$out_json"
