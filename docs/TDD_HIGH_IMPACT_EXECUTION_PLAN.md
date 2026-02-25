# TDD High-Impact Execution Plan

> Issue: #2078 — Coverage & Local-CI Enforcement

## Goal

Make coverage and local-CI standards concrete, visible, and enforced so that every PR demonstrates test-driven development discipline.

## Phases

### Phase 1: Local Coverage Targets

Add `make coverage` and `make coverage-html` to the Makefile so developers can check coverage locally using the same thresholds as CI.

**Deliverables:**
- `Makefile` with `coverage` and `coverage-html` targets
- Uses `cargo-llvm-cov` with the same configuration as CI
- 75% line coverage threshold enforced locally

**Prerequisite:** `cargo install cargo-llvm-cov`

### Phase 2: CI Coverage PR Comments

Add a `rust-coverage.yml` workflow that runs `cargo-llvm-cov` on PRs and posts a sticky comment with the coverage summary. This makes coverage visible without digging into artifacts.

**Deliverables:**
- `.github/workflows/rust-coverage.yml` workflow
- `marocchino/sticky-pull-request-comment` posts coverage table on PRs
- Coverage failures block merge (75% threshold)

### Phase 3: Example TDD Recipes

Document concrete TDD loops so developers know exactly how to use `local-ci` as a TDD driver.

**Deliverables:**
- `docs/LOCAL_CI_VALIDATION.md` with step-by-step TDD recipes
- Covers: writing a test, running `local-ci`, checking coverage, iterating

### Phase 4: Enforcement Gate

Connect local-ci parity to PR gating so the CI workflow and local workflow produce identical results.

**Deliverables:**
- `ci.yml` runs `local-ci` as the single CI job (already done)
- `rust-coverage.yml` adds coverage as a required check
- CLAUDE.md references coverage workflow

## References

- [Local CI Validation](./LOCAL_CI_VALIDATION.md)
- [Contributing Workflow](./CONTRIBUTING_WORKFLOW.md)
- [CLAUDE.md](../CLAUDE.md) — PR and branch workflow
- `.local-ci.toml` — local-ci stage definitions
