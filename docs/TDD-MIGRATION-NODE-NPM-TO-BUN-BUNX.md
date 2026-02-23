# Technical Design: Migrate Node/npm usage to Bun/bunx

## Issue
- Tracking: #71
- Title: Planning & design: migrate from Node/npm to Bun/bunx only

## Goal
Standardize JavaScript/TypeScript tooling entrypoints on Bun so this repository uses:
- `bun install` instead of `npm install`
- `bun run` instead of `npm run`
- `bunx` instead of `npx`

This repository is Rust-first, so the migration focuses on Cloudflare Wrangler invocation paths and operational docs.

## Scope
### In scope
- Local developer command references in repo docs/config comments.
- CI/CD command invocations for Wrangler.
- A baseline inventory of all Node/npm/npx usage in this repository.

### Out of scope
- Changes to Rust runtime behavior.
- Introducing a Node package workspace where none exists today.
- Migrating non-JS toolchains.

## Current State Inventory
Inventory date: 2026-02-23

| Surface | Current state | Risk | Planned action |
| --- | --- | --- | --- |
| `.github/workflows/deploy.yml` | Uses `npx wrangler@3 deploy` | Medium (mixed toolchain, implicit Node dependency) | Install Bun and use `bunx wrangler@3 deploy` |
| `wrangler.toml` comments | Examples use `npx wrangler` | Low (docs drift) | Update examples to `bunx wrangler` |
| `README.md` | Already uses `bunx wrangler deploy` | None | Keep |
| `flake.nix` | Includes `pkgs.bun` | None | Keep |
| package manager files | No `package.json` / lockfile for Node packages | None | Keep Rust-first; no Bun package manifest needed |

## Target State
- No functional usage of `npm`/`npx` in CI or operational docs.
- Bun available in deploy job before Wrangler invocation.
- Wrangler deploy command remains version-pinned (`wrangler@3`) for deterministic behavior.

## Implementation Plan
1. Update `deploy.yml`:
   - Add Bun setup action.
   - Replace `npx wrangler@3 deploy` with `bunx wrangler@3 deploy`.
2. Update `wrangler.toml` command comments to `bunx`.
3. Keep `README.md` Bun guidance as source of truth.
4. Validate with grep scan:
   - Confirm no remaining `npx` operational references outside migration docs.

## Success Criteria
- Deploy workflow does not call `npx`.
- Wrangler command examples in repository config/docs use `bunx`.
- Inventory and rollout plan are documented in this file for future workstreams.

## Rollback Plan
- Revert only workflow command setup and comment updates if deployment regressions occur.
- Fallback command (temporary): `npx wrangler@3 deploy`.

## Follow-up Work
- If/when Node-based tooling is introduced:
  - enforce Bun-only scripts in that workspace (`bun run ...`).
  - add CI guard to fail on `npm`/`npx` commands outside approved migration docs.
- Optionally move from pinned `wrangler@3` to tested newer major version in a separate PR.

## Issue Checklist Status
- [x] Review and refine TDD
- [x] Decide target repo/workspace scope (this repository only; Rust-first)
- [x] Complete inventory (scripts, CI, docs, lockfile, hooks)
- [x] Schedule implementation slices (this PR = slice 1: deploy + docs normalization)
