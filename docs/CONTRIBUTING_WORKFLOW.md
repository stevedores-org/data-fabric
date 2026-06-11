# Contributing Workflow

**Tooling:** This repo uses [Bun](https://bun.sh) for any JS/TS tooling (e.g. `bunx wrangler`). Use Bun only; no Node/npm required. See `.bun-version` for the pinned version.

## Default Workflow (PR-First)

1. Sync often:
```bash
git checkout develop
git pull --ff-only
```

2. Create a focused branch off **`develop`**:
```bash
git checkout -b feat/<short-topic>
```

3. Make small, testable changes.

4. Re-sync before push:
```bash
git fetch origin
git rebase origin/develop
```

5. Push and open a PR **against `develop`** (never `main` for feature/fix work):
```bash
git push -u origin feat/<short-topic>
gh pr create --base develop --head feat/<short-topic>
```

## Team Norms

- **All PRs target `develop`.** `main` is production; merges to `main` happen via release promotion, not direct feature PRs.
- Prefer multiple small PRs over large batches.
- Pull (`git pull --ff-only`) frequently to avoid drift.
- Keep PRs single-purpose with clear scope and evidence.
