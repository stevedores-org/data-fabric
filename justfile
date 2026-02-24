#!/usr/bin/env just --justfile

set dotenv-load := true

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# data-fabric local development & deployment targets
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Display help
help:
    @just --list

# Run all unit tests
test:
    cargo test --lib

# Run unit tests + watch for changes (requires cargo-watch)
test-watch:
    cargo watch -x "test --lib"

# Build the Cloudflare Worker WASM
build:
    cargo install worker-build
    worker-build --release

# Start local Cloudflare Workers development server (requires local D1)
dev-worker: build
    bunx wrangler dev

# Set up local development environment with D1
setup-local:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "🔧 Setting up local data-fabric development environment..."

    # Create local D1 database
    if ! bunx wrangler d1 info data-fabric --local 2>/dev/null; then
        echo "📦 Creating local D1 database..."
        bunx wrangler d1 create data-fabric --local
    fi

    # Apply migrations to local database
    echo "🗂️  Applying migrations to local database..."
    bunx wrangler d1 migrations apply data-fabric --local

    # Create KV namespace locally
    echo "✨ Setting up local KV namespaces..."
    # Note: wrangler dev handles KV automatically for local dev

    echo "✅ Local development environment ready!"
    echo ""
    echo "Next steps:"
    echo "  1. Run tests:  just test"
    echo "  2. Start dev server: just dev-worker"
    echo "  3. Test with oxidizedgraph: just test-integration"

# Initialize D1 with seed data for testing
db-seed:
    bunx wrangler d1 execute data-fabric --local < migrations/0001_ws2_domain_model.sql
    bunx wrangler d1 execute data-fabric --local < migrations/0002_orchestration.sql
    bunx wrangler d1 execute data-fabric --local < migrations/0005_ws8_multi_tenant.sql
    echo "✅ Database seeded with schema"

# Reset local D1 database
db-reset:
    #!/usr/bin/env bash
    echo "⚠️  Resetting local database..."
    rm -f .wrangler/state/v3/d1/data-fabric.sqlite
    echo "✅ Database reset"

# Full clean setup: reset DB and reapply migrations
db-clean-setup: db-reset setup-local
    echo "✅ Clean setup complete"

# Format code
fmt:
    cargo fmt --all

# Lint with clippy
lint:
    cargo clippy --all --target wasm32-unknown-unknown -- -D warnings

# Run formatter and linter checks
check: fmt lint
    @echo "✅ All checks passed"

# Integration test: start worker dev server in background and run tests
test-integration: build
    #!/usr/bin/env bash
    set -euo pipefail

    echo "🚀 Starting data-fabric worker in background..."
    bunx wrangler dev --local > /tmp/data-fabric-dev.log 2>&1 &
    WRANGLER_PID=$!

    # Give worker time to start
    sleep 3

    # Check if worker is running
    if ! kill -0 $WRANGLER_PID 2>/dev/null; then
        echo "❌ Failed to start worker"
        cat /tmp/data-fabric-dev.log
        exit 1
    fi

    echo "✅ Worker started (PID: $WRANGLER_PID)"
    echo "📝 Logs: /tmp/data-fabric-dev.log"

    # Run integration tests
    echo ""
    echo "Running integration tests..."
    FABRIC_URL="http://localhost:8787" cargo test --lib

    # Cleanup
    kill $WRANGLER_PID 2>/dev/null || true
    echo ""
    echo "✅ Integration tests complete"

# Deploy to Cloudflare (requires authentication)
deploy-prod:
    @echo "⚠️  Deploying to production..."
    @echo "Make sure you've run: bunx wrangler login"
    bunx wrangler deploy --env production

# Deploy to staging
deploy-staging:
    @echo "Deploying to staging..."
    bunx wrangler deploy --env staging

# Show worker logs from Cloudflare
logs-remote:
    bunx wrangler tail

# Show local worker logs
logs-local:
    tail -f /tmp/data-fabric-dev.log

# Install dependencies and prepare environment
install:
    bunx wrangler --version
    cargo fetch

# Clean build artifacts
clean:
    cargo clean
    rm -rf build/
    rm -rf .wrangler/

# Full CI suite
ci: check test

# Development server with live reload
dev: setup-local test dev-worker

# Status: show environment info
status:
    @echo "=== data-fabric Status ==="
    @echo "Rust version: $(rustc --version)"
    @echo "Cargo version: $(cargo --version)"
    @echo "Wrangler version: $(bunx wrangler --version 2>/dev/null || echo 'not installed')"
    @echo "Bun version: $(bun --version 2>/dev/null || echo 'not installed')"
    @cargo test --lib --no-run 2>&1 | tail -1 || echo "Tests not yet built"
