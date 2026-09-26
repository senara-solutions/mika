---
title: "fix: dispatch-lib post-groom state machine — GROOMED outcome override + plan validation date-independence"
type: fix
origin: "mika#1394"
created: 2026-06-11
---

# fix: dispatch-lib post-groom state machine — GROOMED outcome override + plan validation date-independence

**Ticket:** mika issue#1394

## Summary

Two interacting bugs in `dispatch-lib.sh` prevent the autonomous groom → impl cascade from draining cleanly on re-dispatch. The plan validation block uses a date-specific find pattern (`${TODAY_PREFIX}-*-plan.md`) that false-negatives on plans created prior days, poisoning RESULT with `PIPELINE_INCOMPLETE`. When `_iterate_groom_loop` subsequently succeeds (GROOMED), the outcome-upgrade sed (`s/Outcome: PLAN_COMMITTED/Outcome: PLAN_GROOMED/`) doesn't match the already-set `PIPELINE_INCOMPLETE`, so the callback delivers the wrong outcome to mika-dev, which never cascades to dev-pilot.

---

## Problem Frame

On re-dispatch of a dev-groom task (triggered by `ready` label re-toggle or milestone cascade after a prior run failed), two gaps compound:

1. **Plan validation date-specificity (root cause):** `VALID_PLAN=$(find ... -name "${TODAY_PREFIX}-*-plan.md" ...)` at line ~867 finds nothing when the plan was committed on a prior day. This triggers a false `PIPELINE FAILURE: dev-groom produced no valid plan file` even though the plan exists on the branch. The `mika#1333` HEAD-unchanged check (line ~677) correctly uses `*-plan.md` (any date), but the subsequent plan validation block overrides with the date-specific pattern.

2. **Outcome upgrade fragility (consequence):** After `_iterate_groom_loop` succeeds, line 2024 runs `sed 's/Outcome: PLAN_COMMITTED/Outcome: PLAN_GROOMED/'`. When RESULT already contains `Outcome: PIPELINE_INCOMPLETE` (from the false plan validation), the sed doesn't match. The callback delivers `PIPELINE_INCOMPLETE` to mika-dev, which doesn't cascade to dev-pilot.

---

## Requirements

- R1. On dev-groom re-dispatch with a plan from a prior day, the plan validation block must not false-positive as PIPELINE FAILURE.
- R2. After `_iterate_groom_loop` succeeds (returns 0), the callback RESULT must contain `Outcome: PLAN_GROOMED` regardless of what earlier checks set.
- R3. Existing first-run behavior (plan created today, HEAD changed) must be preserved.
- R4. Existing PIPELINE_INCOMPLETE behavior on genuine iterate-loop failure must be preserved.

---

## Key Technical Decisions

**KTD-1: Use `_find_issue_plan` for plan validation instead of date-specific find.**
The `_find_issue_plan` function (line 1164) already handles issue-scoped plan lookup with filename-pattern + content-fallback. Reusing it in the plan validation block eliminates the date-specificity bug and keeps plan-location logic in one place. The >500-byte filter is already built into `_find_issue_plan`.

**KTD-2: Unconditional outcome override on iterate-loop success.**
Replace the sed-based outcome upgrade with a direct string replacement that matches ANY `Outcome: ...` line. This makes the GROOMED outcome resilient to prior RESULT pollution from any check (plan validation, HEAD-unchanged, PR-existence, etc.).

---

## Scope Boundaries

### In scope
- Fix plan validation date-specificity in `_run_claude_pilot` post-flight block
- Fix outcome upgrade after `_iterate_groom_loop` success in `dispatch_claude_pilot`

### Out of scope / deferred
- Changes to `/mika-groom-plan-only` pilot behavior (the pilot's "plan exists" early-exit is correct — `_iterate_groom_loop` runs after the pilot regardless)
- Changes to `_iterate_groom_loop` internal logic (the state machine itself is correct; the bug is in the surrounding RESULT handling)
- Changes to mika-dev's callback interpretation

---

## Implementation Units

### U1. Fix plan validation to use `_find_issue_plan` instead of date-specific find

**Goal:** Eliminate the false PIPELINE FAILURE on re-dispatch with plans from prior days.

**Requirements:** R1, R3

**Dependencies:** None

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (modify plan validation block, lines ~866–903)

**Approach:** Replace the date-specific `VALID_PLAN` find with a call to `_find_issue_plan`. The function already:
- Matches by issue number (primary) or content reference (fallback)
- Applies the >500-byte size filter
- Returns the absolute path on success

The `CE_PLAN_INVOKED` log-grep check remains as diagnostic enrichment (demoted to advisory per mika#1303), but the plan-file-existence gate uses `_find_issue_plan` instead of the date-specific find.

**Patterns to follow:** The `_iterate_groom_loop`, `_launch_revise_pilot`, and `_write_canonical_callout` functions all use `_find_issue_plan` for plan location — this change aligns the validation block with the same pattern.

**Test scenarios:**
- First-run (plan created today, issue number in filename): VALID_PLAN set, no false PIPELINE FAILURE
- Re-dispatch (plan created yesterday, issue number in filename): VALID_PLAN set, no false PIPELINE FAILURE
- Re-dispatch (plan with date-prefix slug-tail, issue number in content header): VALID_PLAN set via content fallback
- Genuine missing plan (no plan file on branch): VALID_PLAN empty, PIPELINE FAILURE fires correctly
- Non-dev-groom skill: validation block skipped (existing guard preserved)

### U2. Fix outcome override to unconditionally set PLAN_GROOMED on iterate-loop success

**Goal:** Ensure the callback RESULT contains `Outcome: PLAN_GROOMED` when `_iterate_groom_loop` returns 0, regardless of prior RESULT content.

**Requirements:** R2, R4

**Dependencies:** U1 (reduces false PIPELINE_INCOMPLETE, but U2 is the safety net)

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (modify post-iterate-loop success block, lines ~2021–2024)

**Approach:** Replace the sed `'s/Outcome: PLAN_COMMITTED/Outcome: PLAN_GROOMED/'` with a sed that matches any `Outcome: .*` line and replaces it with `Outcome: PLAN_GROOMED`. If no `Outcome:` line exists (edge case), append it. Also strip any preceding `PIPELINE FAILURE:` lines from RESULT since the iterate loop's success supersedes them — the canonical callout was written, the grooming is complete.

For the else branch (iterate-loop failure), the existing pattern is acceptable: it prepends PIPELINE FAILURE and adjusts the outcome. But the sed should also match `Outcome: PIPELINE_INCOMPLETE` (which may already be set by the plan validation block) to avoid double-outcome lines.

**Patterns to follow:** The existing outcome classification block (lines ~937–963) sets outcomes once; the iterate-loop adjustment should be a clean override, not a layered sed.

**Test scenarios:**
- First-run GROOMED: RESULT has PLAN_COMMITTED → sed replaces with PLAN_GROOMED
- Re-dispatch GROOMED: RESULT has PIPELINE_INCOMPLETE → sed replaces with PLAN_GROOMED
- Re-dispatch GROOMED with "Note: HEAD unchanged" prefix: PIPELINE FAILURE lines stripped, outcome set to PLAN_GROOMED
- Iterate-loop failure: RESULT gets PIPELINE_INCOMPLETE (existing behavior preserved)
- Iterate-loop failure on already-PIPELINE_INCOMPLETE RESULT: no double outcome lines

**Verification:** After applying both fixes, a dev-groom re-dispatch where `_iterate_groom_loop` succeeds should produce a callback RESULT containing `Outcome: PLAN_GROOMED` and NO `PIPELINE FAILURE:` prefix lines.
