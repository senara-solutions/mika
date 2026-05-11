---
module: qa-review
tags: [qa-pipeline, source-changes, exclude-list, false-positive]
problem_type: heuristic-drift
category: workflow-issues
date: 2026-05-11
issue: 975
---

# QA Pipeline: docs/brainstorms/ Missing from Source-Changes Exclude List

## Problem

The QA review skill's "source changes exist" check had an incomplete exclude list. It filtered out `docs/plans/`, `docs/solutions/`, and `.claude/` — but not `docs/brainstorms/`, `docs/adr/`, or `.github/`. This caused false-positive `block[pipeline]` on brainstorm-only PRs, creating a circular dependency (a brainstorm doc would need a plan doc describing the addition of a brainstorm doc).

## Root Cause

Two parallel implementations of the "is this a source change?" heuristic drifted apart:

- `scripts/verify-pipeline.sh` correctly excluded all `docs/` subdirectories via `grep -v -E '^docs/'`
- `skills/bundled/qa-review/system_prompt.md` only excluded specific subdirectories (`docs/plans/`, `docs/solutions/`)

The QA skill check was added before `docs/brainstorms/` existed as a convention and was never updated when that directory came into use.

## Fix

Replaced the individual `docs/plans/` and `docs/solutions/` exclusions in the QA review prompt with a single `^docs/` exclusion, aligning with `verify-pipeline.sh`'s existing `SOURCE_BUCKET` logic. Also added `\.github/` to the Step 2 check where it was missing (already present in the pipeline-exempt check).

**File:** `skills/bundled/qa-review/system_prompt.md` — three edits (pipeline-exempt check, Step 2 description, Step 2 command).

## Lesson

When two components implement the same heuristic (CI script + skill prompt), keep them in sync. The broader pattern (`^docs/`) is more future-proof than enumerating subdirectories — new `docs/` subdirectories are automatically excluded without requiring a prompt update.
