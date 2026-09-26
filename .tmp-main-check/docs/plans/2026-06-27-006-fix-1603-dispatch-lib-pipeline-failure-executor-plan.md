# Plan: fix(dispatch-lib): rewrite PIPELINE FAILURE 'executor mode' diagnostic messages

**Ticket:** mika issue#1603
**Type:** fix (diagnostic wording)
**Scope:** single-file change in `skills/bundled/_shared/dispatch-lib.sh` + test update in `test-dispatch-lib.sh`

## Problem

When `_find_issue_plan` returns empty, `dispatch-lib.sh` emits PIPELINE FAILURE messages that assert the cause is "pilot drifted into executor mode." This is misleading when the actual failure is `_find_issue_plan`'s discovery logic missing a real plan file (e.g., header shape mismatch — mika#1602 class). The founding incident (session `9bce2bc0`, mika#1600 dispatch) cost ~30 extra minutes of debugging because the message pointed at pilot behavior when the bug was in discovery code.

Two messages need rewriting:

1. **Line 1089** (both checks failed: no plan file AND no `/ce:plan` invocation): currently says "Session drifted into executor mode."
2. **Line 1094** (plan missing but `/ce:plan` was called or log unavailable): currently says "Session likely drifted into executor mode."

## Solution

Replace both messages with diagnostic text that:
- Names the **observable state** (`_find_issue_plan` returned empty, describing what it checked)
- Lists **candidate causes** (pilot drift OR discovery regex miss) without asserting one
- Tells the reader **how to falsify** cause (a) — check for `docs/plans/*-plan.md >500 bytes` in the worktree directly

No behavior change — same `RESULT` variable assignment, same failure exit, same downstream handling.

## Changes

### 1. `skills/bundled/_shared/dispatch-lib.sh` — lines 1087–1096

**Line 1089 (both checks failed):**

Current:
```
RESULT="PIPELINE FAILURE: dev-groom produced no valid plan file (no issue-scoped plan >500 bytes found via _find_issue_plan for $REPO#$ISSUE_NUM) and no /ce:plan invocation detected in session log. Session drifted into executor mode.
```

New:
```
RESULT="PIPELINE FAILURE: dev-groom: _find_issue_plan returned empty for $REPO#$ISSUE_NUM (no filename match *-${ISSUE_NUM}-*-plan.md and no header-line match in first 20 lines for known prefixes) and no /ce:plan invocation detected in session log. Likely causes: (a) pilot drifted into executor mode without writing a plan, (b) plan was written but _find_issue_plan's regex didn't match the header shape — check \${WORKTREE_DIR}/docs/plans/*-plan.md >500 bytes to distinguish (see mika#1602 class).
```

**Line 1094 (plan missing, /ce:plan called or unknown):**

Current:
```
RESULT="PIPELINE FAILURE: dev-groom produced no valid plan file (no issue-scoped plan >500 bytes found via _find_issue_plan for $REPO#$ISSUE_NUM). Session likely drifted into executor mode.
```

New:
```
RESULT="PIPELINE FAILURE: dev-groom: _find_issue_plan returned empty for $REPO#$ISSUE_NUM (no filename match *-${ISSUE_NUM}-*-plan.md and no header-line match in first 20 lines for known prefixes). Inspect \${WORKTREE_DIR}/docs/plans/*-plan.md >500 bytes directly — if a plan exists, this is a _find_issue_plan discovery bug (see mika#1602 class); if no plan exists, the pilot drifted into executor mode.
```

### 2. `skills/bundled/_shared/test-dispatch-lib.sh` — line 2232

The structural test at line 2232 asserts on the string `'Session drifted into executor mode'` for branch-ordering verification. This grep needs updating to match text present in the new messages.

**Current (line 2232):**
```bash
DRIFT_MSG_LINE=$(echo "$DRIFT_BLOCK" | grep -n 'Session drifted into executor mode' | head -1 | cut -d: -f1)
```

**New:**
```bash
DRIFT_MSG_LINE=$(echo "$DRIFT_BLOCK" | grep -n 'pilot drifted into executor mode' | head -1 | cut -d: -f1)
```

The phrase "pilot drifted into executor mode" appears in both new messages (as one of the candidate causes), so the branch-ordering assertion still works: it verifies the POLICY_DENY branch comes before any drift-mentioning branch.

## Files touched

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Rewrite two PIPELINE FAILURE message strings (~lines 1089, 1094) |
| `skills/bundled/_shared/test-dispatch-lib.sh` | Update grep pattern in branch-ordering structural test (~line 2232) |

## Acceptance criteria

- [x] AC1. The "both checks failed" PIPELINE FAILURE message names both candidate causes (drift OR regex miss) and tells the reader how to falsify (a) by checking the worktree's plan dir.
- [x] AC2. The "plan missing but /ce:plan called" PIPELINE FAILURE message names what `_find_issue_plan` actually checked (filename pattern + header prefixes) and tells the reader how to distinguish a discovery bug from genuine pilot drift.
- [x] AC3. Existing tests in `test-dispatch-lib.sh` that assert on these messages either continue to pass or are updated to assert the new wording.
- [x] AC4. No behavior change beyond message text — same failure exit, same `RESULT` variable, same downstream handling.

## Risk

Minimal. Pure diagnostic wording change. No control flow, exit code, or variable-assignment changes. The only regression surface is the structural test grep pattern, which is updated in lockstep.

## Out of scope

- The `_find_issue_plan` regex itself (mika#1602).
- Adding structured telemetry fields (`failure_class=discovery|drift|unknown`) — ticket's tier-2 escalation path, not this PR.
