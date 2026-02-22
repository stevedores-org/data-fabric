# Contributing Workflow

## Default Workflow (PR-First)

1. Sync often:
```bash
git checkout main
git pull --ff-only
```

2. Create a focused branch:
```bash
git checkout -b feat/<short-topic>
```

3. Make small, testable changes.

4. Re-sync before push:
```bash
git fetch origin
git rebase origin/main
```

5. Push and open a PR:
```bash
git push -u origin feat/<short-topic>
gh pr create --base main --head feat/<short-topic>
```

## Team Norms

- Open PRs often, even for small increments.
- Prefer multiple small PRs over large batches.
- Pull (`git pull --ff-only`) frequently to avoid drift.
- Keep PRs single-purpose with clear scope and evidence.
