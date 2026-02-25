.PHONY: help test build dev-worker setup-local db-reset db-clean-setup fmt lint check test-integration deploy-prod deploy-staging logs-remote logs-local install clean ci dev status coverage coverage-html

# Default target
.DEFAULT_GOAL := help

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# data-fabric Build & Development Targets
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

help:
	@grep -E '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-20s\033[0m %s\n", $$1, $$2}'

test: ## Run all unit tests (70 tests)
	cargo test --lib

test-watch: ## Run tests with watch (requires cargo-watch)
	cargo watch -x "test --lib"

build: ## Build the Cloudflare Worker WASM
	cargo install worker-build
	worker-build --release

dev-worker: build ## Start local Cloudflare Workers dev server
	bunx wrangler dev --local

setup-local: ## Set up local development environment with D1
	@echo "🔧 Setting up local data-fabric development environment..."
	@if ! bunx wrangler d1 info data-fabric --local 2>/dev/null; then \
		echo "📦 Creating local D1 database..."; \
		bunx wrangler d1 create data-fabric --local; \
	fi
	@echo "🗂️  Applying migrations to local database..."
	@bunx wrangler d1 migrations apply data-fabric --local
	@echo "✅ Local development environment ready!"
	@echo ""
	@echo "Next steps:"
	@echo "  1. Run tests: make test"
	@echo "  2. Start dev server: make dev-worker"

db-seed: ## Initialize D1 with seed data
	bunx wrangler d1 execute data-fabric --local < migrations/0001_ws2_domain_model.sql
	bunx wrangler d1 execute data-fabric --local < migrations/0002_orchestration.sql
	bunx wrangler d1 execute data-fabric --local < migrations/0005_ws8_multi_tenant.sql
	@echo "✅ Database seeded with schema"

db-reset: ## Reset local D1 database
	@echo "⚠️  Resetting local database..."
	@rm -f .wrangler/state/v3/d1/data-fabric.sqlite
	@echo "✅ Database reset"

db-clean-setup: db-reset setup-local ## Full clean setup: reset DB and reapply migrations
	@echo "✅ Clean setup complete"

fmt: ## Format code
	cargo fmt --all

lint: ## Lint with clippy
	cargo clippy --all --target wasm32-unknown-unknown -- -D warnings

check: fmt lint ## Run formatter and linter checks
	@echo "✅ All checks passed"

test-integration: build ## Integration test with worker running in background
	@echo "🚀 Starting data-fabric worker in background..."
	@bunx wrangler dev --local > /tmp/data-fabric-dev.log 2>&1 &
	@WRANGLER_PID=$$!; \
	sleep 3; \
	if ! kill -0 $$WRANGLER_PID 2>/dev/null; then \
		echo "❌ Failed to start worker"; \
		cat /tmp/data-fabric-dev.log; \
		exit 1; \
	fi; \
	echo "✅ Worker started (PID: $$WRANGLER_PID)"; \
	echo "📝 Logs: /tmp/data-fabric-dev.log"; \
	echo ""; \
	echo "Running integration tests..."; \
	FABRIC_URL="http://localhost:8787" cargo test --lib; \
	TEST_RESULT=$$?; \
	kill $$WRANGLER_PID 2>/dev/null || true; \
	echo ""; \
	echo "✅ Integration tests complete"; \
	exit $$TEST_RESULT

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Coverage Targets (requires: cargo install cargo-llvm-cov)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

COV_THRESHOLD := 75

coverage: ## Run coverage and print summary (75% threshold)
	cargo llvm-cov --summary-only --fail-under-lines $(COV_THRESHOLD)

coverage-html: ## Generate HTML coverage report and open it
	cargo llvm-cov --html
	@echo "Opening coverage report..."
	@open target/llvm-cov/html/index.html 2>/dev/null || xdg-open target/llvm-cov/html/index.html 2>/dev/null || echo "Report at target/llvm-cov/html/index.html"

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Deployment & Infrastructure
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

deploy-prod: ## Deploy to Cloudflare production
	@echo "⚠️  Deploying to production..."
	@echo "Make sure you've run: bunx wrangler login"
	bunx wrangler deploy --env production

deploy-staging: ## Deploy to staging environment
	@echo "Deploying to staging..."
	bunx wrangler deploy --env staging

logs-remote: ## Show worker logs from Cloudflare
	bunx wrangler tail

logs-local: ## Show local worker logs
	@tail -f /tmp/data-fabric-dev.log 2>/dev/null || echo "No logs yet - start dev-worker first"

install: ## Install dependencies and prepare environment
	bunx wrangler --version
	cargo fetch

clean: ## Clean build artifacts
	cargo clean
	rm -rf build/
	rm -rf .wrangler/

ci: check test ## Full CI suite

dev: setup-local test dev-worker ## Development server with live reload

docker-build: ## Build OCI image with Docker
	docker build -f Containerfile -t data-fabric:latest .

podman-build: ## Build OCI image with Podman (rootless)
	podman build -f Containerfile -t data-fabric:latest .

docker-up: ## Start with docker-compose
	docker-compose up -d

podman-compose-up: ## Start with podman-compose
	podman-compose up -d

docker-down: ## Stop docker-compose services
	docker-compose down

podman-compose-down: ## Stop podman-compose services
	podman-compose down

docker-logs: ## View docker-compose logs
	docker-compose logs -f worker

podman-logs: ## View podman-compose logs
	podman-compose logs -f worker

status: ## Show environment info
	@echo "=== data-fabric Status ==="
	@echo "Rust version: $(shell rustc --version)"
	@echo "Cargo version: $(shell cargo --version)"
	@echo "Wrangler version: $(shell bunx wrangler --version 2>/dev/null || echo 'not installed')"
	@echo "Bun version: $(shell bun --version 2>/dev/null || echo 'not installed')"
	@cargo test --lib --no-run 2>&1 | tail -1 || echo "Tests not yet built"
