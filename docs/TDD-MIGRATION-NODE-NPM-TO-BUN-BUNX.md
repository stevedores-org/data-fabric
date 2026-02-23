# TDD: Migration from Node/npm/npx to Bun/bunx

Issue: #71

## 1. Objective

Migrate JS tooling usage from `npm`/`npx` to `bun`/`bunx` for local development, CI, and docs, while keeping runtime semantics stable.

## 2. Target Repo/Workspace Decision (Feature 2)

Decision: **single target repo: `stevedores-org/data-fabric` (this repo), no Node workspace split required.**

Rationale:
- The project is Rust-first and not a JS monorepo.
- There is no `package.json`-driven workspace topology to migrate.
- Bun migration scope is tooling entrypoints only (Wrangler/dev scripts/docs/CI command paths).

Decision details:
- Target repositories: `1` (`stevedores-org/data-fabric` only).
- Target JS workspaces: `0` (none required).
- Critical-path package manager: `bun` only.
- Command runner replacement:
  - `npm install` -> `bun install`
  - `npm run <script>` -> `bun run <script>`
  - `npx <tool>` -> `bunx <tool>`

Non-goals for this decision:
- No runtime rewrite from Rust to JS/TS.
- No new workspace splitting by package manager.

## 3. Inventory Checklist (to execute next)

- [ ] Local scripts and wrappers (`README`, helper scripts, Make targets)
- [ ] CI workflows invoking Node/npm/npx
- [ ] Hook/tooling entrypoints requiring package runner changes
- [ ] Lockfile policy (`bun.lock` canonical; remove npm lock usage in critical path)

## 4. Success Criteria (test-driven)

- [ ] All documented install/run commands use Bun/bunx only
- [ ] CI passes without npm/npx commands in active workflows
- [ ] Developer quickstart validates with Bun-only path on clean machine
- [ ] Rollback documented and tested (restore npm-based command paths)

## 5. Rollback

- Keep migration in isolated PR(s).
- If breakage occurs, revert Bun command-path commits and restore previous CI/docs command invocations.

## 6. Implementation Order

1. Commit this repo/workspace decision record.
2. Complete command inventory across docs + workflows.
3. Apply Bun command-path updates in small PR slices.
4. Validate CI and local quickstart.

## 7. Closing Checklist

- [ ] Issue #71 checklist item "Decide target repo(s) / workspaces" marked complete
- [ ] Inventory completed and linked
- [ ] Follow-up implementation PRs opened
