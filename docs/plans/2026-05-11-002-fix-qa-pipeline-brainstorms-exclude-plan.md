---
type: fix
ticket: mika issue#975
branch: fix/975/qa-pipeline-add-docs-brainstorms-to
date: 2026-05-11
---

# Fix QA Pipeline: Add docs/brainstorms/ to Source-Changes Exclude List

## Problem

The QA review skill's "Source changes exist" check (Step 2, line 102 of `skills/bundled/qa-review/system_prompt.md`) has an incomplete exclude list. It only filters out `docs/plans/`, `docs/solutions/`, and `.claude/` — but misses `docs/brainstorms/`, `docs/adr/`, `.github/`, and other non-source paths.

This creates a false-positive `block[pipeline]` when a PR contains only brainstorm documents, because they're incorrectly classified as "source changes" requiring a plan callout.

The CI-side `scripts/verify-pipeline.sh` already handles this correctly by excluding all `docs/` subdirectories (via `grep -v -E '^docs/'`), `.github/`, and `.claude/worktrees/`. The QA skill prompt needs to align.

## Root Cause Analysis

Two parallel implementations of the "is this a source change?" heuristic have drifted:

| Implementation | Excludes | Correct? |
|---|---|---|
| `scripts/verify-pipeline.sh` (lines 60-64) | All `docs/`, `.github/`, `.claude/worktrees/` | ✅ |
| `qa-review/system_prompt.md` check 2 (line 102) | `docs/plans/`, `docs/solutions/`, `.claude/` only | ❌ |
| `qa-review/system_prompt.md` pipeline-exempt check (line 87) | `docs/plans/`, `docs/solutions/`, `.claude/`, `.github/` | Partially ✅ (still missing `docs/brainstorms/` etc.) |

## Fix

Align both QA prompt checks with the `verify-pipeline.sh` logic. Replace individual `docs/plans/` and `docs/solutions/` exclusions with a single `docs/` exclusion, and add `.github/` where missing.

### Change 1: Step 2 "Source changes exist" (line 100-103)

**Before:**
```
run_gh("pr diff <PR_URL> --name-only | grep -v '^docs/plans/' | grep -v '^docs/solutions/' | grep -v '^\\.claude/' | head -1")
```

**After:**
```
run_gh("pr diff <PR_URL> --name-only | grep -v '^docs/' | grep -v '^\\.github/' | grep -v '^\\.claude/' | head -1")
```

This aligns with `verify-pipeline.sh`'s `SOURCE_BUCKET` logic: source = everything NOT under `docs/`, `.github/`, or `.claude/`.

### Change 2: Pipeline-exempt docs-only confirmation (line 87)

**Before:**
```
run_gh("pr diff <PR_URL> --name-only | grep -v '^docs/plans/' | grep -v '^docs/solutions/' | grep -v '^\\.claude/' | grep -v '^\\.github/' | head -1")
```

**After:**
```
run_gh("pr diff <PR_URL> --name-only | grep -v '^docs/' | grep -v '^\\.github/' | grep -v '^\\.claude/' | head -1")
```

Same broadening — if the only changes are under `docs/` (any subdirectory), `.github/`, or `.claude/`, that's docs-only.

### Change 3: Update Step 2 description text (line 100)

**Before:**
```
2. **Source changes exist** — Check that the PR has changes beyond `docs/plans/`, `docs/solutions/`, and `.claude/`:
```

**After:**
```
2. **Source changes exist** — Check that the PR has changes beyond `docs/`, `.github/`, and `.claude/`:
```

## Files Modified

1. `skills/bundled/qa-review/system_prompt.md` — Three changes (lines 87, 100, 102)

## Acceptance Criteria

- [ ] `docs/brainstorms/` is excluded from the source-changes check in the QA pipeline
- [ ] A PR containing only `docs/brainstorms/` files passes QA without a `block[pipeline]` on missing plan callout
- [ ] Existing excludes (`docs/plans/`, `docs/solutions/`, `.claude/`) are preserved (subsumed by broader `docs/` exclude)
- [ ] The pipeline-exempt docs-only check is also aligned
- [ ] `scripts/verify-pipeline.sh` logic and `qa-review` prompt logic agree on what constitutes "source changes"

## Risk Assessment

**Low risk.** The change broadens the exclude list, which can only reduce false-positive blocks (never introduce false-negative passes). The broader `docs/` pattern is already proven correct in `verify-pipeline.sh`. No Rust code changes — prompt-only fix, takes effect on next skill bundle rebuild.
