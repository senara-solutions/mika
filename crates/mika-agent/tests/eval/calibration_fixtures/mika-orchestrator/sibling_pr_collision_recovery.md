# PR convergence — #1623 will not merge

PR #1623 was green and qa-APPROVED an hour ago. Now it will not merge. Here is
the current state from `gh pr view 1623 --json ...`:

```json
{
  "number": 1623,
  "title": "feat(kg): add corpus drift probe",
  "reviewDecision": "APPROVED",
  "mergeStateStatus": "DIRTY",
  "baseRefName": "main",
  "headRefName": "feat/1618/kg-corpus-drift-probe"
}
```

Recent merges on `main` (from `git log --oneline -5 origin/main`):

```
c4e21a90  feat(db): schema v40 — kg_corpus_drift table (#1624)   <- merged 34 min ago
a76b09ba  fix(gateway): retry classification for 502 (#1619)
f0d18e22  chore(ci): pin actions to SHAs (#1615)
```

Notes:

- PR #1623 adds a migration test that asserts against `pragma_table_info` for the
  KG tables. It was authored against `main` at `a76b09ba`.
- PR #1624 (`#1624`, merged 34 minutes ago) shipped schema v40 and a new
  `kg_corpus_drift` table — it changed the same schema surface #1623's test
  asserts against.
- #1623's branch has not been updated since #1624 merged.

Why did #1623 go DIRTY, and what is the recovery? Name the specific PR that moved
the base and the corrective action.
