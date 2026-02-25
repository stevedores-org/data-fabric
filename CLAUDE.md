# CLAUDE.md

## PR and Branch Workflow

- Base branch is always `develop`.
- Pull from remote frequently to avoid stale branches and merge drift.
- Open pull requests frequently in small increments.
- Keep pull requests atomic: one concern per PR, no mixed-scope changes.

## Notes

- Prefer SSH remotes when possible for Git operations.

## Task Shorthand

- `crr`: code review requested (findings-first; severity-ordered as `HIGH`/`MEDIUM`/`LOW`).
- `acr`: address code review feedback in the target PR.
- `ffc`: fix failing checks in the target PR.
- `fmc`: fix merge conflicts in the target PR.
- `btf`: build the requested feature end-to-end and open a PR.
- `sm`: squash merge when approved and no follow-up changes are needed.

## Local CI & Coverage

- Run `local-ci` before every push. Configuration: `.local-ci.toml`.
- Run `make coverage` to check line coverage locally (75% threshold).
- Run `make coverage-html` to generate and open an HTML coverage report.
- See [docs/LOCAL_CI_VALIDATION.md](docs/LOCAL_CI_VALIDATION.md) for TDD recipes.
- See [docs/TDD_HIGH_IMPACT_EXECUTION_PLAN.md](docs/TDD_HIGH_IMPACT_EXECUTION_PLAN.md) for the coverage enforcement plan.
