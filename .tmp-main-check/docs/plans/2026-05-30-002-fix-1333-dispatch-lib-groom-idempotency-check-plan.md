---
title: "fix: Groom idempotency check fires on architect-pending plan (mid-flow short-circuit)"
type: fix
status: active
date: 2026-05-30
---

# fix: Groom idempotency check fires on architect-pending plan (mid-flow short-circuit)

## Overview

When a groom dispatch commits a plan but the architect first-pass never runs (or `_iterate_groom_loop` fails), the dispatch result is classified as `PLAN_GROOMED` because the plan file exists on branch. mika-dev's wrapper task sees this success-shaped outcome and marks the groom completed, leaving the ticket in a half-groomed state with no architect verdict or body callouts. Re-dispatching hits the same gate because the plan is already on branch.

## Problem Frame

Observed 2026-05-29 on mika#949: `dispatch_claude_pilot` ran the pilot (which committed the plan), then `_iterate_groom_loop` either failed or was skipped, but the outcome classification at line 699 produced `Outcome: PLAN_GROOMED` based solely on plan-file existence. The wrapper task completed with reasoning "plan already committed and pushed... Pipeline reported HEAD unchanged." The architect first-pass never ran, no body callouts were injected, and the ticket sits indistinguishable from `pending-dispatch`.

Three root causes converge:

1. **`_iterate_groom_loop` failure is silently tolerated** (line 1615): `_iterate_groom_loop || echo "info: ..."` swallows the exit code. No PIPELINE FAILURE marker is appended to RESULT.

2. **Outcome classification doesn't gate on architect verdict** (lines 699-707): `PLAN_GROOMED` is emitted whenever `$VALID_PLAN` is set, regardless of whether `_iterate_groom_loop` succeeded. The name "PLAN_GROOMED" implies architect review happened, but only plan-file existence is checked.

3. **No re-dispatch recovery** for the architect-pending state: When re-dispatched, the pilot finds the plan already on branch, produces no new commits, HEAD-unchanged fires as PIPELINE FAILURE (line 454), but `_iterate_groom_loop` runs again. If it succeeds this time, the canonical callout is written and the groom converges. If it fails again, the same silent-tolerance loop repeats. The issue is that mika-dev's wrapper never gets a signal that distinguishes "architect didn't run" from "groom completed."

## Requirements Trace

- R1. A groom dispatch that ends with plan-on-branch but no architect verdict in GH body callouts is treated as INCOMPLETE, not COMPLETED (from acceptance criteria)
- R2. Re-dispatching such a ticket resumes the groom from the architect pass without re-writing the plan (from acceptance criteria)
- R3. The audit_event reasoning on incomplete grooms is precise — "architect pass did not complete" not "HEAD unchanged" (from acceptance criteria)
- R4. The `_iterate_groom_loop` state machine contract from mika#1271 is preserved
- R5. The operator-facing `/mika-groom-ticket` flow is unaffected

## Scope Boundaries

- All changes are in `skills/bundled/_shared/dispatch-lib.sh`
- The Rust-side grooming gate (`check_grooming_markers` in `executor.rs`) is unchanged — it correctly requires all three signals (branch, plan, verdict) and already rejects architect-pending tickets
- `/mika-groom-ticket` (operator-direct) is unchanged — it runs its own architect calls inline
- `_write_canonical_callout` idempotency check is unchanged — it correctly gates on all three signals including verdict

## Context & Research

### Relevant Code and Patterns

- `dispatch_claude_pilot()` (line 1494): main entry — calls `_run_claude_pilot`, then `_iterate_groom_loop`, then `_push_branch`, then `_deliver_callback`
- `_run_claude_pilot()` outcome classification (lines 691-712): the `PLAN_GROOMED` arm at line 699 is the direct cause of the false-positive success signal
- `_iterate_groom_loop()` (line 1288): returns 0 on GROOMED, 1 on all failure paths — the return code is the signal
- Line 1615: `_iterate_groom_loop || echo "info: ..."` — the silent tolerance point
- `_write_canonical_callout()` (line 1178): already has correct three-signal idempotency — only writes when architect verdict is present. Not the bug.
- mika#1267 / mika#1296: analogous fix for dev-pilot HEAD-unchanged idempotency — same "too-aggressive idempotency" failure class

### Institutional Learnings

- The dev-pilot HEAD-unchanged fix (mika#1267) established the pattern: post-flight checks must distinguish "work already done correctly" from "work incomplete." The groom side needs the same treatment.
- The `Outcome:` line in RESULT is the load-bearing signal that mika-dev's wrapper task uses to classify success/failure. Changing the outcome from `PLAN_GROOMED` to `PIPELINE_INCOMPLETE` when architect didn't converge is the minimal fix.

## Key Technical Decisions

- **Propagate `_iterate_groom_loop` exit code into RESULT**: When the loop returns non-zero, append a PIPELINE FAILURE marker to RESULT before outcome classification runs. This ensures the `grep -qF "PIPELINE FAILURE:"` check at line 691 wins over the `PLAN_GROOMED` arm. Rationale: follows the existing error-propagation pattern used by all other post-flight checks (HEAD-unchanged, plan-validation, PR-existence).

- **Split outcome into `PLAN_COMMITTED` vs `PLAN_GROOMED`**: Rename the plan-only outcome to `PLAN_COMMITTED` (plan exists but architect verdict unknown) and reserve `PLAN_GROOMED` for post-architect-convergence. However, since the PIPELINE FAILURE marker already wins over any outcome classification, this is a clarity improvement, not a behavioral fix. The structural fix is the PIPELINE FAILURE propagation.

- **Re-dispatch recovery via architect-only resume**: When `_iterate_groom_loop` finds a valid plan on branch but `_run_claude_pilot` produced HEAD-unchanged (plan already committed from prior run), the HEAD-unchanged PIPELINE FAILURE for dev-groom should be downgraded to an informational note (not a failure). The plan already exists — that's the expected state for re-dispatch. The architect pass is what matters, and `_iterate_groom_loop` will attempt it. Only if the architect pass also fails should PIPELINE FAILURE fire.

## Open Questions

### Resolved During Planning

- **Should `_iterate_groom_loop` be made retry-aware?** No — the loop already runs on every dispatch (including re-dispatch). The fix is making the outcome classification aware of the loop's result, not adding retry logic inside the loop.
- **Should the HEAD-unchanged check be skipped for dev-groom?** No — but for dev-groom, HEAD-unchanged when a valid plan already exists on branch is expected on re-dispatch, not a failure. The check should be gated: HEAD-unchanged + plan-on-branch + dev-groom = informational note, not PIPELINE FAILURE.

### Deferred to Implementation

- Exact wording of the PIPELINE FAILURE message for architect-convergence failure — should be precise enough for mika-dev's reasoning but follows existing message patterns

## Implementation Units

- [x] **Unit 1: Propagate `_iterate_groom_loop` failure into RESULT**

**Goal:** When `_iterate_groom_loop` returns non-zero, append a PIPELINE FAILURE marker to RESULT so the outcome classification produces `PIPELINE_INCOMPLETE` instead of `PLAN_GROOMED`.

**Requirements:** R1, R3

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**
Replace the silent-tolerance pattern at line 1615:
```
_iterate_groom_loop || echo "info: ..."
```
with a pattern that captures the exit code and, on failure, prepends a PIPELINE FAILURE marker to RESULT. The marker must appear before the outcome classification block (lines 691-712) runs — but since `_iterate_groom_loop` runs AFTER `_run_claude_pilot` returns (which is where outcome classification lives), the RESULT has already been classified. Therefore, the fix must re-classify: after `_iterate_groom_loop` fails, overwrite or append a new Outcome line. The simplest approach: set a flag variable (e.g., `GROOM_CONVERGED=0/1`) and check it after the loop, appending `PIPELINE FAILURE: architect convergence did not complete` and replacing the existing `Outcome:` line with `Outcome: PIPELINE_INCOMPLETE`.

Key detail: the `Outcome:` line is appended inside `_run_claude_pilot` at lines 691-712. After `_iterate_groom_loop` runs (line 1614-1615), RESULT already contains an Outcome line. The fix must either:
  - (a) Replace the existing Outcome line via sed/parameter expansion, or
  - (b) Append a second, overriding failure block that mika-dev's reasoning will prioritize

Option (b) is simpler and follows the existing pattern where PIPELINE FAILURE markers are prepended and the first `grep -qF "PIPELINE FAILURE:"` wins. But RESULT is already finalized. Option (a) using bash string replacement (`${RESULT/Outcome: PLAN_GROOMED/Outcome: PIPELINE_INCOMPLETE}`) is cleaner and more explicit.

**Patterns to follow:**
- HEAD-unchanged PIPELINE FAILURE pattern at lines 451-457
- The `grep -qF "PIPELINE FAILURE:"` priority check at line 691

**Test scenarios:**
- Happy path: `_iterate_groom_loop` returns 0 (architect GROOMED) — RESULT retains `Outcome: PLAN_GROOMED` unchanged
- Error path: `_iterate_groom_loop` returns 1 (architect failed/ESCALATE/guard failure) — RESULT contains `PIPELINE FAILURE: architect convergence` and `Outcome: PIPELINE_INCOMPLETE`
- Edge case: `_iterate_groom_loop` returns 1 but RESULT already contains `PIPELINE FAILURE:` from HEAD-unchanged check — no double-classification; architect failure marker is still appended for diagnostic precision (R3)

**Verification:**
- After this unit, a groom dispatch where `_iterate_groom_loop` fails produces `Outcome: PIPELINE_INCOMPLETE` in the callback result, not `Outcome: PLAN_GROOMED`

- [x] **Unit 2: Downgrade HEAD-unchanged to informational for dev-groom with plan-on-branch**

**Goal:** When dev-groom re-dispatch finds HEAD unchanged but a valid plan already exists on branch, treat this as expected re-dispatch state (informational note) rather than PIPELINE FAILURE. The architect pass is what matters — Unit 1's `_iterate_groom_loop` propagation handles that.

**Requirements:** R2, R3

**Dependencies:** Unit 1

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**
At the HEAD-unchanged check (lines 451-457), add a dev-groom guard: when `$SKILL = dev-groom` AND a valid plan file exists on the branch (check `$VALID_PLAN` or re-run the plan-file discovery), downgrade from PIPELINE FAILURE to an informational note in RESULT. This allows re-dispatch to proceed to `_iterate_groom_loop` (which runs the architect pass) without the HEAD-unchanged PIPELINE FAILURE poisoning the outcome.

Sequencing detail: the HEAD-unchanged check at line 451-457 runs BEFORE the plan-validation block at line 615. So `$VALID_PLAN` is not yet set at line 451. Two options:
  - (a) Move plan-file discovery earlier (before HEAD-unchanged check) — invasive
  - (b) Inline a lightweight plan-file existence check at line 451 — `find "$WORKTREE_DIR/docs/plans" -name "*-plan.md" -size +500c 2>/dev/null | head -1` — duplicates the pattern but is self-contained

Option (b) is preferred: the pattern is a one-liner, already used in three other places (`_run_claude_pilot` line 617, `_iterate_groom_loop` line 1320, `_write_canonical_callout` line 1222), and avoids restructuring the post-flight block.

**Patterns to follow:**
- The existing dev-pilot-only guard at line 464 (`if [ "$SKILL" = "dev-pilot" ]`)
- Plan-file discovery pattern used at lines 617, 1222, 1320

**Test scenarios:**
- Happy path: dev-groom re-dispatch, plan exists on branch, HEAD unchanged — RESULT contains informational note (not PIPELINE FAILURE), `_iterate_groom_loop` runs and handles architect pass
- Happy path: dev-groom first dispatch, no prior plan, HEAD unchanged (plan was committed in this run so HEAD actually changed) — this scenario doesn't trigger the guard because HEAD changed
- Edge case: dev-groom re-dispatch, plan does NOT exist on branch (corrupted worktree), HEAD unchanged — PIPELINE FAILURE fires as before (plan-file guard does not match)
- Edge case: dev-pilot dispatch, HEAD unchanged — behavior unchanged (guard is dev-groom only)

**Verification:**
- Re-dispatching a groom on a ticket with plan-on-branch but no architect verdict does not produce `PIPELINE FAILURE: HEAD unchanged` — it produces an informational note and proceeds to the architect pass

- [x] **Unit 3: Rename `PLAN_GROOMED` outcome to `PLAN_COMMITTED` for pre-architect state**

**Goal:** Make the outcome naming precise: `PLAN_COMMITTED` when plan exists but architect verdict is unknown; `PLAN_GROOMED` only after architect convergence. This satisfies R3 (precise audit reasoning) and eliminates the semantic confusion that caused mika-dev to interpret plan-committed as plan-groomed.

**Requirements:** R3

**Dependencies:** Unit 1

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**
At the outcome classification block (lines 699-707), rename `PLAN_GROOMED` to `PLAN_COMMITTED` in the base case. After `_iterate_groom_loop` succeeds (Unit 1's success path), upgrade to `PLAN_GROOMED`. This gives mika-dev two distinct signals:
  - `Outcome: PLAN_COMMITTED` — plan file exists, architect pass pending or in progress
  - `Outcome: PLAN_GROOMED` — plan file exists AND architect converged on GROOMED

The `PLAN_GROOMED` upgrade happens in the post-`_iterate_groom_loop` success path added by Unit 1. The existing classification at line 699 emits `PLAN_COMMITTED`. If the loop succeeds, the outcome is replaced with `PLAN_GROOMED`.

Also update the `_iterate_groom_loop` success path to emit the upgrade. On GROOMED convergence (lines 1359-1363 and 1410-1414), after `_write_canonical_callout` succeeds, the loop returns 0. The caller (line 1614) then replaces the outcome.

**Patterns to follow:**
- The existing `Outcome: PR_OPENED`, `Outcome: PIPELINE_INCOMPLETE`, `Outcome: UNKNOWN` naming at lines 695-712

**Test scenarios:**
- Happy path: plan committed, architect GROOMED — final RESULT contains `Outcome: PLAN_GROOMED`
- Error path: plan committed, architect failed — final RESULT contains `Outcome: PIPELINE_INCOMPLETE` (from Unit 1), not `PLAN_COMMITTED`
- Edge case: plan committed, `_iterate_groom_loop` skipped (not dev-groom skill) — impossible; only dev-groom reaches this path

**Verification:**
- The string `PLAN_GROOMED` only appears in RESULT when `_iterate_groom_loop` returned 0

## System-Wide Impact

- **Interaction graph:** The `Outcome:` line in RESULT is consumed by mika-dev's wrapper task reasoning. Changing from `PLAN_GROOMED` to `PLAN_COMMITTED` (or `PIPELINE_INCOMPLETE` on failure) changes how mika-dev classifies the dispatch result. mika-dev already handles `PIPELINE_INCOMPLETE` — it surfaces the failure instead of marking completed.
- **Error propagation:** `_iterate_groom_loop` failures currently vanish at line 1615. After this fix, they propagate into RESULT as PIPELINE FAILURE markers, which flow through `_deliver_callback` to mika-dev.
- **State lifecycle risks:** Re-dispatch recovery (Unit 2) introduces a path where HEAD-unchanged is not a failure. This is safe because `_iterate_groom_loop` (which runs the architect) is the actual completion gate, and Unit 1 ensures its failure is propagated.
- **Unchanged invariants:** `_write_canonical_callout` idempotency is unchanged. `check_grooming_markers` in executor.rs is unchanged. `/mika-groom-ticket` operator flow is unchanged. The `_iterate_groom_loop` state machine (first-pass, READY/ITERATE/ESCALATE, second-pass, GROOMED) is unchanged — only the handling of its exit code changes.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| mika-dev may not handle `PLAN_COMMITTED` outcome gracefully | `PLAN_COMMITTED` is only emitted transiently — Unit 1 ensures `PIPELINE_INCOMPLETE` replaces it when architect fails. When architect succeeds, it's replaced with `PLAN_GROOMED`. mika-dev never sees `PLAN_COMMITTED` as a final outcome in practice. |
| Bash string replacement (`${RESULT/old/new}`) may fail on multi-line RESULT | Use `sed` or a more robust replacement pattern. Test with actual multi-line RESULT strings. |

## Sources & References

- Related issues: mika#1333 (this ticket), mika#1267 (analogous dev-pilot fix), mika#1296 (dev-pilot dirty-worktree), mika#1271 (iterate-loop state machine)
- Related code: `skills/bundled/_shared/dispatch-lib.sh` lines 451-457, 615-654, 691-712, 1288-1431, 1494-1662
- Related code: `crates/mika-agent/src/skills/executor.rs` `check_grooming_markers()` (line 800) — unchanged, reference only
