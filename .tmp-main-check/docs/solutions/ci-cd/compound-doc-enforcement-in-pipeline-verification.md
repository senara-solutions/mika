---
title: "Compound doc enforcement in pipeline verification"
category: ci-cd
date: 2026-04-05
tags: [pipeline, ci, verify-pipeline, compound, enforcement]
modules: [scripts]
issue: "#327"
---

# Compound doc enforcement in pipeline verification

## Problem

The `/mika` pipeline's `scripts/verify-pipeline.sh` checked for plan docs (Check 1) and source changes (Check 2) but had no check for compound docs in `docs/solutions/`. This meant `/ce:compound` (step 7) could be skipped without detection — both locally and in CI. Discovered when task mika#321's claude-pilot session skipped `/ce:compound` entirely and CI passed.

## Root Cause

The original artifact-driven enforcement design (PR #230, documented in `docs/solutions/cross-repo-patterns/2026-03-21-artifact-driven-pipeline-enforcement.md`) deliberately made `/ce:compound` optional with the rationale: "Not every change produces lessons worth documenting." As the pipeline matured and compound docs became expected for every PR, the enforcement didn't evolve to match.

## Solution

Added Check 3 to `scripts/verify-pipeline.sh` following the existing pattern:

```bash
# Check 3: Compound doc in docs/solutions/
COMPOUND=$(echo "$ALL" | grep '^docs/solutions/.*\.md$' || true)
if [[ -z "$COMPOUND" ]]; then
  echo "MISSING: No compound doc in docs/solutions/. Run /ce:compound." >&2
  ERRORS=$((ERRORS + 1))
fi
```

Also updated `.claude/commands/mika.md` step 8 to include `/ce:compound` in the recovery instructions, ensuring agents can self-recover from Check 3 failures.

## Key Decisions

1. **Hard gate, not warning.** Matches the enforcement pattern of Checks 1 and 2. The original "optional" design decision was superseded by pipeline maturity.
2. **No skip mechanism.** Release branches are already exempted by CI's `if` guard. All `/mika` pipeline PRs go through `/ce:compound` at step 7, so the check is always satisfiable.
3. **No CI changes needed.** The `pipeline-artifacts` job already runs `verify-pipeline.sh origin/main` — new checks are picked up automatically.
4. **Agent recovery parity.** The `/mika` command step 8 was updated to list `/ce:compound` alongside `/ce:plan` and `/ce:work` so agents have explicit recovery paths for all three checks.

## Prevention

- When adding new pipeline steps that produce artifacts, immediately add a corresponding check to `verify-pipeline.sh` and update the `/mika` command's recovery instructions.
- The pattern is: grep for file path → if-empty → error with actionable command → increment error counter.
