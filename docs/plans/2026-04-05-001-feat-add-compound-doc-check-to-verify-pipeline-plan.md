---
title: "feat: Add compound doc check to verify-pipeline.sh"
type: feat
status: completed
date: 2026-04-05
---

# feat: Add compound doc check to verify-pipeline.sh

## Overview

Add a third verification check to `scripts/verify-pipeline.sh` that ensures a compound doc exists in `docs/solutions/` before PR creation. This closes the enforcement gap where `/ce:compound` could be skipped without detection — both locally and in CI.

## Problem Statement / Motivation

`scripts/verify-pipeline.sh` currently checks for:
1. Plan doc in `docs/plans/` → enforces `/ce:plan`
2. Source code changes beyond the plan → enforces `/ce:work`

It does **not** check for a compound doc in `docs/solutions/`, meaning `/ce:compound` (step 7 of the `/mika` pipeline) can be skipped without any enforcement. In task mika#321, the claude-pilot session skipped `/ce:compound` entirely and the CI Pipeline Artifacts check passed anyway.

The original artifact-driven enforcement design (see `docs/solutions/cross-repo-patterns/2026-03-21-artifact-driven-pipeline-enforcement.md`) explicitly called `/ce:compound` optional: _"Not every change produces lessons worth documenting."_ Issue #327 reverses this decision — the pipeline has matured and compound docs are now expected for every PR that goes through the `/mika` pipeline.

## Proposed Solution

### 1. Add Check 3 to `scripts/verify-pipeline.sh`

Follow the existing pattern (grep → if-empty → error → increment):

```bash
# Check 3: Compound doc in docs/solutions/
COMPOUND=$(echo "$ALL" | grep '^docs/solutions/.*\.md$' || true)
if [[ -z "$COMPOUND" ]]; then
  echo "MISSING: No compound doc in docs/solutions/. Run /ce:compound." >&2
  ERRORS=$((ERRORS + 1))
fi
```

### 2. Update success message

Add compound doc path to the existing success output:

```bash
echo "Pipeline verification passed. Plan: $PLAN Compound: $COMPOUND"
```

### 3. Update script header comments

Add check 3 to the comment block at the top of the script.

### Files to modify

- `scripts/verify-pipeline.sh` — add Check 3, update header comments and success message

## Technical Considerations

- **No CI changes needed.** The `pipeline-artifacts` job in `.github/workflows/ci.yml` already runs `verify-pipeline.sh origin/main`. The new check is picked up automatically.
- **Grep pattern:** `^docs/solutions/.*\.md$` matches any `.md` file anywhere under `docs/solutions/`, including all 14 existing subdirectories. Both new and modified files satisfy the check (via `git diff --name-only`).
- **Check 2 interaction:** Compound docs under `docs/solutions/` are NOT excluded from Check 2's `CODE` filter (which only strips `docs/plans/` and `.claude/`). This is correct — a compound doc alone without source changes would still fail Check 2.
- **Release branch exemption:** Already handled by CI's `if` guard that skips `release-plz-*` and `release/*` branches.
- **Cross-repo scope:** This change targets `mika/` only. The same script exists in `mika-cloud/`, `mika-platform/`, and `claude-pilot/` — those can be updated separately if desired.

## Acceptance Criteria

- [x] `scripts/verify-pipeline.sh` includes Check 3 for compound docs in `docs/solutions/`
- [x] Script header comments list all 3 checks
- [x] Success message includes compound doc path
- [x] Script exits with error code 1 when compound doc is missing
- [x] Existing checks 1 and 2 continue to work unchanged
- [x] Error message follows existing pattern: `"MISSING: No compound doc in docs/solutions/. Run /ce:compound."`

## Sources & References

- Related issue: #327
- Institutional learning: `docs/solutions/cross-repo-patterns/2026-03-21-artifact-driven-pipeline-enforcement.md`
- CI job: `.github/workflows/ci.yml` lines 102-114 (`pipeline-artifacts` job)
- CI docs-sync pattern: `docs/solutions/ci-cd/ci-gate-crate-local-docs-sync-drift.md`
