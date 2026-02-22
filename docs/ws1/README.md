# WS1: Source Extraction Baseline

This folder implements issue #42 (WS1: Source Extraction - Baseline Inventory & Architecture Seed).

## Deliverables
- `docs/ws1/SOURCE_INVENTORY_MATRIX.md`: module-level inventory of `lornu.ai` data-fabric sources.
- `docs/ws1/CAPABILITY_GAP_MAP.md`: extracted capabilities vs target needs, with priority scores.
- `docs/ws1/MIGRATION_RISK_REGISTER.md`: migration risks with severity, likelihood, and mitigations.
- `docs/ws1/ARCHITECTURE_SEED.md`: service-boundary seed aligned to WS2 canonical entities.

## Scope
- Primary extraction target: `lornu.ai/crates/data-fabric`.
- Pattern extraction support: `lornu.ai/crates/lornu-data`, `lornu.ai/apps/zero-copy-connector`, and orchestration/MCP crates.

## Outcome
WS1 provides a known-good baseline to reduce architecture churn before implementation-heavy workstreams (WS3-WS6).
