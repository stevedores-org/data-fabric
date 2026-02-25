.PHONY: coverage coverage-html

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
