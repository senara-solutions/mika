# Plan: Add calibration rules for issue number verification and task UUID hygiene

**Issue:** mika#684
**Date:** 2026-04-20
**Type:** Enhancement (prompt change only)

## Problem

Two fabrication patterns observed on 2026-04-20:
1. mika-dev said "mika#675 complete" when she meant mika#682 — confused related issue numbers from memory
2. mika-dev tried `check_task` with a memorized UUID that had drifted — the real task ID was different

Both are the same class: trusting memorized references instead of verifying against live data.

## Solution

Add two new calibration rules to `skills/bundled/self-dev/system_prompt.md`:

### Rule 10 — Verify issue numbers before completion claims

Never cite an issue number from memory when reporting completion. Cross-reference against the active task's label or `check_task` output. Related issues with similar numbers (e.g., #675 vs #682) are a known confusion source.

**Incident:** 2026-04-20 — reported "mika#675 complete" when the completed issue was mika#682.

### Rule 11 — Never memorize task UUIDs

Never store task UUIDs in core memory. Store the issue reference (e.g. mika#677). Look up the UUID fresh from `list_tasks` every time you need it. UUIDs drift across sessions and compaction.

**Incident:** 2026-04-20 — `check_task` failed with wrong UUID. Engine dedup guard prevented duplicate task creation.

## Changes

| File | Change |
|------|--------|
| `skills/bundled/self-dev/system_prompt.md` | Add Rule 10 and Rule 11 after Rule 9 in Calibration Rules section |

## Risks

- None — prompt-only change, no code impact
- Rules are additive (new numbered rules after existing Rule 9)
