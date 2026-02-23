# data-fabric

Cloudflare-native, Rust-first data fabric for orchestrated autonomous AI agent builders.

## Mission
Increase agent-builder velocity with reusable RAG context, durable provenance, and low-friction task communication.

## Current Status
- Rust Cloudflare Worker scaffold in place
- Lean architecture and mission plan documented

## Quick Start
```bash
cargo test
# deploy flow (after wrangler auth)
# worker-build --release
# bunx wrangler deploy
```
**Tooling:** We use [Bun](https://bun.sh) for JS/TS tooling (e.g. `bunx wrangler`). No Node/npm required; install Bun and use `bunx` for one-off commands.

## Docs
- `docs/ARCHITECTURE.md`
- `docs/MISSION_PLAN.md`
- `docs/CONTRIBUTING_WORKFLOW.md`
- `docs/TDD-MIGRATION-NODE-NPM-TO-BUN-BUNX.md`
- `docs/TDD-BUN-BUNX-EXECUTION-PLAN.md`
- `docs/ws1/README.md`
- `docs/WS5_RETRIEVAL_MEMORY.md`
- `docs/WS4_POLICY_GOVERNANCE.md`
