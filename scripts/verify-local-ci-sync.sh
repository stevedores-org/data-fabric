#!/usr/bin/env bash
set -euo pipefail

CI_FILE=".github/workflows/ci.yml"
LOCAL_CI_FILE=".local-ci.toml"

check_contains() {
  local file="$1"
  local pattern="$2"
  local message="$3"

  if ! rg -Fq "$pattern" "$file"; then
    echo "sync-check failed: ${message}" >&2
    exit 1
  fi
}

check_contains "$CI_FILE" "run: cargo fmt --all -- --check" \
  "missing fmt cargo command in CI workflow"
check_contains "$CI_FILE" "run: cargo clippy --all-targets -- -D warnings" \
  "missing clippy cargo command in CI workflow"
check_contains "$CI_FILE" "run: cargo check --target wasm32-unknown-unknown" \
  "missing wasm cargo check command in CI workflow"
check_contains "$CI_FILE" "run: cargo test" \
  "missing cargo test command in CI workflow"

check_contains "$LOCAL_CI_FILE" "[stages.fmt]" \
  "local-ci fmt stage missing"
check_contains "$LOCAL_CI_FILE" "[stages.clippy]" \
  "local-ci clippy stage missing"
check_contains "$LOCAL_CI_FILE" "[stages.check]" \
  "local-ci check stage missing"
check_contains "$LOCAL_CI_FILE" "[stages.test]" \
  "local-ci test stage missing"
check_contains "$LOCAL_CI_FILE" 'cmd = ["cargo", "check", "--target", "wasm32-unknown-unknown"]' \
  "local-ci check command must target wasm32-unknown-unknown"

echo "local-ci sync-check: ok"
