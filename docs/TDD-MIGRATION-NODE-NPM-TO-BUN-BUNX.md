# TDD: Migration from Node/npm to Bun/bunx Only

**Status:** Draft  
**Scope:** Replace Node.js + npm (and npx) with Bun runtime + Bun package manager + bunx for all install, script, and tooling usage.  
**Out of scope:** Changing runtime semantics (Bun remains Node-compatible); migrating non–Node/npm parts of the repo.

---

## 1. Goals

- **Single stack:** All installs, scripts, and CLI tool invocations use Bun (bun install, bun run, bunx).
- **No npm/node in critical path:** No `npm install`, `npm run`, or `npx` in local dev, CI, or docs.
- **Reproducible:** Lockfile and scripts yield identical behavior on Bun; CI and contributors use Bun only.
- **Validation:** Migration success defined by tests and checklist; no regressions in build, test, or run.

---

## 2. Success Criteria (Test-Driven)

Before considering the migration done, the following must hold.

| ID | Criterion | How to verify |
|----|-----------|----------------|
| S1 | Install from clean clone with Bun only | `rm -rf node_modules && bun install` succeeds; no npm invoked. |
| S2 | All package scripts run via Bun | `bun run <script>` for every script in root and workspaces; no `npm run`. |
| S3 | All one-off tools run via bunx | No `npx` in scripts, CI, or contributing docs; use `bunx` (or `bun run` with local deps). |
| S4 | Lockfile is Bun-native | `bun.lockb` present and used; remove or stop using `package-lock.json` for install. |
| S5 | CI uses Bun | CI jobs install and run with Bun; no Node/npm setup for primary workflow. |
| S6 | Docs and CONTRIBUTING reference Bun | No “npm install” or “npx” in setup/run instructions. |
| S7 | Existing test suite passes | Same tests (e.g. vitest) run with `bun run test` (or equivalent) and pass. |
| S8 | Build artifacts unchanged | Build output (bundles, types) matches pre-migration where applicable. |

These can be turned into concrete tests or CI checks where possible (e.g. script that fails if `npm` or `npx` appears in certain files).

---

## 3. Assumptions and Constraints

- **Bun version:** Pin a minimum Bun version (e.g. in CI and in CONTRIBUTING) to avoid drift.
- **Node compatibility:** Bun’s Node compatibility is sufficient for existing code and deps; no Node-only native addons that Bun cannot run.
- **Workspaces:** If the repo uses npm workspaces, migrate to Bun workspaces (same `workspaces` in root `package.json`; Bun supports it).
- **Tooling:** Vitest, ESLint, Prettier, and similar run under Bun; use `bunx vitest`, `bunx eslint`, etc., or `bun run` scripts that invoke them.
- **Optional:** Keep a `.nvmrc` or `engines.node` for any remaining Node-based fallback (e.g. external integrators); primary path is Bun-only.

---

## 4. Inventory (Pre-Migration)

Capture before starting:

- [x] **Scripts:** List every `npm run <script>` and `npx <tool>` used in repo and CI. — *None in repo; only CI deploy used `npx wrangler@3 deploy`.*
- [x] **CI:** Identify all jobs that run `npm install`, `npm run`, or `npx`; note where Node version is set. — *Deploy workflow: was `npx wrangler@3 deploy`; migrated to Bun + `bunx wrangler@3 deploy` (first feature).*
- [x] **Docs:** Grep for `npm`, `npx`, `node` in README, CONTRIBUTING, and other docs. — *README already references `bunx wrangler deploy` for local.*
- [x] **Lockfile:** Confirm current lockfile (e.g. `package-lock.json`) and whether any tooling depends on it. — *No Node lockfile; repo is Rust-first. Bun used only for wrangler in CI.*
- [x] **Hooks:** Pre-commit or other hooks that call npm/npx; switch to bun/bunx. — *N/A; no npm/npx in hooks.*

---

## 5. Migration Steps (Ordered)

1. **Bun in CI (optional early step)**  
   Add a CI job that uses Bun: install with `bun install`, run tests with `bun run test`. Keep existing Node job until migration is validated. Ensures Bun is supported in the environment.

2. **Local lockfile and install**  
   Run `bun install` at repo root (and in workspaces if applicable). Commit `bun.lockb`. Optionally keep `package-lock.json` in a separate commit so it can be reverted; once stable, remove it and document “use Bun only.”

3. **Scripts in package.json**  
   Replace any inline `npx` or `npm` usage in scripts with `bunx` or `bun run`. Example: `"test": "vitest run"` with vitest as dependency, or `"lint": "bunx eslint ."` if preferred. Ensure `bun run build`, `bun run test`, etc., work.

4. **Pre-commit and hooks**  
   Update hooks (e.g. Husky, lint-staged) to use `bun run` / `bunx` instead of npm/npx.

5. **CI migration**  
   Switch primary install/run to Bun: set up Bun in the runner, `bun install --frozen-lockfile`, `bun run test`, `bun run build`. Remove or deprecate Node-only jobs once S1–S8 hold.

6. **Documentation**  
   Update README, CONTRIBUTING, and any runbooks: “Install: bun install”, “Run tests: bun run test”, “One-off: bunx &lt;tool&gt;”. Add minimum Bun version and link to Bun install guide.

7. **Engines and hints**  
   Add `"packageManager": "bun@<version>"` in root `package.json` if desired; optionally document “Bun only” in CONTRIBUTING.

8. **Cleanup**  
   Remove `package-lock.json` from tree and from any CI that might regenerate it; add a check or note that the repo is Bun-only.

---

## 6. Rollback

- Keep a branch or tag at pre-migration state.
- If lockfile reverted: restore `package-lock.json`, switch CI and docs back to npm; revert script and hook changes.
- No data or API contract changes are implied; rollback is repo and CI only.

---

## 7. Risks and Mitigations

| Risk | Mitigation |
|------|-------------|
| Dependency resolution differs (Bun vs npm) | Run full test suite and compare build output; fix or pin any differing dep. |
| CI runner has no Bun | Use official Bun install step or a Bun action; pin Bun version. |
| Contributors still use Node/npm | Document “Bun only” and provide Bun install link; optional fallback doc for “run with Node” at their own risk. |
| Native addons | Verify under Bun; if a dep is Node-only, replace or keep Node for that subpath only (out of scope for “Bun only” if not acceptable). |

---

## 8. Checklist Before Closing TDD

- [x] Inventory (section 4) completed and recorded.
- [x] All success criteria (section 2) verified and documented (this repo: no Node lockfile/scripts; deploy uses Bun only).
- [x] CONTRIBUTING (or equivalent) updated to Bun-only.
- [x] CI runs on Bun; no npm in critical path.
- [x] Rollback plan (section 6) agreed and branch/tag available.

---

## 9. References

- [Bun package manager](https://bun.sh/docs/cli/install)
- [Bun run / bunx](https://bun.sh/docs/cli/run)
- [Bun workspaces](https://bun.sh/docs/install/workspaces) (if applicable)
