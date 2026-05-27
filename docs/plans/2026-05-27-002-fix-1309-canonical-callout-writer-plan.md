---
title: "fix: canonical-callout writer doesn't fire on architect GROOMED"
type: fix
status: active
date: 2026-05-27
---

# fix: canonical-callout writer doesn't fire on architect GROOMED

## Overview

`_write_canonical_callout` in dispatch-lib silently fails to write the grooming summary callout to GitHub issue bodies after architect GROOMED verdicts. The iterate-groom-loop reaches GROOMED (verdict trail proves it), calls the writer, the writer fails, the `||` non-fatal handler swallows the failure, and the loop returns 0. The dispatch gate then refuses to advance because the three required body signals are absent.

Observed on mika#1305, #806, #736, #716 on 2026-05-27 — each with multiple GROOMED cycles in the verdict trail and zero callouts in the issue body.

## Problem Frame

The `_iterate_groom_loop` → `_write_canonical_callout` path has zero observability between "loop converged on GROOMED" and "callout appeared in body." Three gaps compound:

1. **stderr suppression:** `gh issue edit --body-file "$tmpfile" 2>/dev/null` (line 1197) hides the actual failure reason.
2. **Non-fatal error handling:** The `||` on lines 1280-1281 and 1331-1332 treats callout-write failure as non-fatal — the loop returns 0 even when the callout never landed.
3. **No verification:** The writer trusts `gh issue edit`'s exit code without re-reading the body to confirm the signals are present.

Without observability, we cannot determine the root cause. The fix adds structured logging, captures stderr, verifies the write, and propagates failure so the operator sees it.

## Requirements Trace

- R1. Operator can diagnose exactly where `_write_canonical_callout` fails from structured log events
- R2. `gh issue edit` stderr is captured and logged, not suppressed
- R3. Callout write is verified by re-reading the body after the write
- R4. Callout-write failure propagates to RESULT for operator visibility in the callback
- R5. Plan-file find pattern is resilient to naming convention drift

## Scope Boundaries

- Single file: `skills/bundled/_shared/dispatch-lib.sh`
- No Rust changes
- No changes to `/mika-groom-ticket` or `/mika-groom-plan-only` slash commands
- No changes to the plan-file naming convention in `/ce:plan`

## Context & Research

### Relevant Code and Patterns

- `_write_canonical_callout` (lines 1105-1206): the writer function
- `_iterate_groom_loop` (lines 1208-1351): the state machine that calls the writer on GROOMED
- `_push_branch` (lines 670-716): runs after the loop, pushes commits — has good stderr capture pattern to follow
- `_run_claude_pilot` (lines ~380-630): post-flight checks use `TODAY_PREFIX` date-based find pattern as fallback
- `_setup_gh_auth` (lines 165-183): sets up gh CLI auth before the loop runs
- `_trail_append` (lines 972+): verdict-trail writer — same execution context as the callout writer, proves the loop reaches GROOMED

### Institutional Learnings

- dispatch-lib's `2>/dev/null` pattern on `gh` CLI calls has caused silent failures before (mika#1283 — `_arch_ask` was passing literal `@path` strings that gh doesn't expand, diagnosed only after adding observability)
- The post-flight plan validation (line 546) uses `${TODAY_PREFIX}-*-plan.md` (date-based), while `_iterate_groom_loop` and `_write_canonical_callout` use `*-${ISSUE_NUM}-*-plan.md` (issue-number-based). This asymmetry is a known fragility point — if `/ce:plan` generates a filename without the issue number, the issue-scoped find fails

## Key Technical Decisions

- **Capture stderr to variable, not file:** Use `2>&1` capture into a local variable via command substitution wrapper, avoiding tmpfile proliferation. The stderr output from `gh issue edit` is small (usually a single error line).
- **Post-write verification as a separate check:** Re-read the body via `gh issue view` after `gh issue edit` returns 0. This catches silent API failures, race conditions, and body-format issues that `gh` doesn't surface via exit code.
- **Find-pattern fallback, not replacement:** Keep the issue-scoped pattern as primary (it's more precise when it works), but fall back to `${TODAY_PREFIX}-*-plan.md` (date-based, same as post-flight check) if the issue-scoped pattern finds nothing. This handles the naming-convention drift without changing the convention.
- **Structured logging to stderr, not separate log file:** dispatch-lib runs under `set -x` and stderr is the established logging channel. Structured event names (`canonical_callout_write_start`, `canonical_callout_write_complete`, `canonical_callout_write_error`, `canonical_callout_verify_failed`) use the same naming convention as existing dispatch-lib events.

## Implementation Units

- [ ] **Unit 1: Add structured logging and stderr capture to `_write_canonical_callout`**

**Goal:** Make every execution path through the writer visible to the operator.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**
- Add `canonical_callout_write_start` log event at function entry with `stage`, `session_id`, `REPO`, `ISSUE_NUM`, `BRANCH`
- Remove `2>/dev/null` from `gh issue view` (line 1162) and `gh issue edit` (line 1197) — capture stderr to local variables instead
- On `gh issue view` failure: log the captured stderr alongside the existing WARN
- On `gh issue edit` success: emit `canonical_callout_write_complete` with `stage`, `session_id`
- On `gh issue edit` failure: emit `canonical_callout_write_error` with `stage`, `session_id`, and the captured stderr content
- On idempotency skip: emit existing skip message (already present, no change needed)
- On plan-file not found: log the find pattern and `$WORKTREE_DIR/docs/plans/` directory listing for diagnosis

**Patterns to follow:**
- `_push_branch` (line 704) captures `gh push` stderr to a tmpfile and logs it on failure — same pattern but using variable capture instead of tmpfile since `gh issue edit` stderr is small

**Test scenarios:**
- Happy path: `gh issue edit` succeeds — verify `canonical_callout_write_start` and `canonical_callout_write_complete` events appear in stderr
- Error path: `gh issue edit` fails — verify `canonical_callout_write_error` event appears with the stderr content
- Error path: `gh issue view` fails — verify stderr content is logged, not swallowed
- Edge case: plan file not found — verify directory listing is logged for diagnosis

**Verification:**
- Running `_write_canonical_callout` with intentionally invalid auth produces a log line containing the actual `gh` CLI error message, not just "gh issue edit failed"

- [ ] **Unit 2: Add post-write verification to `_write_canonical_callout`**

**Goal:** Detect and report cases where `gh issue edit` returns 0 but the body doesn't actually contain the expected signals.

**Requirements:** R3

**Dependencies:** Unit 1

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**
- After `gh issue edit` returns 0, re-read the body via `gh issue view` using the same three-signal check the idempotency guard uses (lines 1169-1172)
- If all three signals are present: emit `canonical_callout_write_verified` and return 0
- If any signal is missing: emit `canonical_callout_verify_failed` with which signals are missing, log a truncated excerpt of the actual body (first 500 chars), and return 1
- This transforms a silent data loss into a diagnosable failure

**Patterns to follow:**
- The idempotency check at lines 1169-1177 already implements the three-signal grep — reuse the same pattern for verification

**Test scenarios:**
- Happy path: signals present after write — verify `canonical_callout_write_verified` event
- Error path: signals missing after write — verify `canonical_callout_verify_failed` event with missing signal names
- Edge case: `gh issue view` fails during verification — log warning but still return 0 (the write succeeded per `gh issue edit`; verification is observability, not a gate)

**Verification:**
- If `gh issue edit` returns 0 but the body is unchanged (e.g., GitHub API silently rejected the write), the function now returns 1 and logs which signals are missing

- [ ] **Unit 3: Add plan-file find-pattern fallback**

**Goal:** Handle plan files whose names don't include the issue number.

**Requirements:** R5

**Dependencies:** None (can be done in parallel with Unit 1-2)

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**
- In `_write_canonical_callout` (line 1151) and `_iterate_groom_loop` (line 1240): if the issue-scoped pattern `*-${ISSUE_NUM}-*-plan.md` finds nothing, fall back to `${TODAY_PREFIX}-*-plan.md` (same pattern the post-flight check at line 546 uses)
- Log when the fallback fires: `plan_file_fallback: issue-scoped pattern missed, using date-based fallback`
- Extract into a shared helper `_find_plan_file()` to eliminate the duplicated find logic across three call sites (lines 546, 1151, 1240)

**Patterns to follow:**
- Post-flight plan check at line 546 uses `${TODAY_PREFIX}-*-plan.md` — this is the fallback pattern

**Test scenarios:**
- Happy path: issue-scoped pattern matches — helper returns it directly, no fallback log
- Edge case: issue-scoped pattern misses, date-based pattern matches — helper returns it, emits fallback log
- Edge case: both patterns miss — helper returns empty, caller handles the failure
- Edge case: date-based pattern matches multiple files — helper returns the most recent (sort -r | head -1)

**Verification:**
- A plan file named `2026-05-27-001-feat-ollama-tool-calling-plan.md` (no issue number) is found by the helper when `ISSUE_NUM=1305`

- [ ] **Unit 4: Propagate callout-write failure to RESULT**

**Goal:** Make callout-write failures visible in the callback delivery so the operator sees them without reading dispatch-lib stderr.

**Requirements:** R4

**Dependencies:** Unit 1

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`

**Approach:**
- In the GROOMED branches of `_iterate_groom_loop` (lines 1278-1283 and 1328-1333): when `_write_canonical_callout` returns non-zero, append a structured marker to `RESULT`:
  ```
  CALLOUT WRITE FAILED: canonical callout did not land on $REPO#$ISSUE_NUM body. Dispatch gate will not advance without manual intervention.
  ```
- Keep the loop's return value at 0 (the groom itself succeeded — the plan is committed, the architect approved it). The failure is in the body-write, not the grooming.
- The operator (or mika-dev) sees the marker in the callback result and can manually write the callout via `gh issue edit`

**Patterns to follow:**
- `_push_branch` appends `Push: FAILED` to `RESULT` on push failure (line 712-713) — same pattern

**Test scenarios:**
- Happy path: callout write succeeds — no marker in RESULT
- Error path: callout write fails — RESULT contains `CALLOUT WRITE FAILED:` marker with the repo and issue number

**Verification:**
- When `_write_canonical_callout` fails, the callback result delivered to mika-dev contains the `CALLOUT WRITE FAILED:` marker

## System-Wide Impact

- **Interaction graph:** `_write_canonical_callout` → `gh issue edit` → GitHub API → issue body → `check_grooming_markers` (Rust, executor.rs line 800). The verification step adds a `gh issue view` call after the write, adding one extra API call per successful write.
- **Error propagation:** Failures now propagate through: (1) structured stderr events for log analysis, (2) RESULT marker for callback delivery, (3) verification check return code. The loop still returns 0 on callout-write failure (grooming succeeded; body-write is a delivery concern).
- **API surface parity:** No changes to the three-signal check in `check_grooming_markers` (Rust) or the idempotency check in `_write_canonical_callout` (shell). Both continue to check the same three signals.
- **Unchanged invariants:** The `/mika-groom-ticket` operator-facing flow is unchanged. The iterate-groom-loop state machine transitions are unchanged. The verdict-trail format is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Post-write verification adds an extra `gh issue view` API call per successful write | One extra read call per groom dispatch is negligible. Rate limiting risk is low (GitHub allows 5000 req/hr for authenticated requests). |
| Date-based fallback pattern matches a plan file from a different issue groomed on the same day | The fallback is scoped to `$WORKTREE_DIR/docs/plans/` — worktrees are issue-specific, so only plans committed on that branch are visible. False matches are unlikely. |
| Changing stderr handling could break `set -x` trace output | stderr capture uses command substitution (`err=$(cmd 2>&1 1>&3)` fd-redirect pattern), which doesn't interfere with `set -x` output. The xtrace output goes to fd 2 but is written by bash itself, not captured by the subshell. |

## Sources & References

- Related issues: mika#1309, mika#1271 (contract refactor), mika#1303 (substrate noise cleanup)
- Related code: `skills/bundled/_shared/dispatch-lib.sh` lines 1105-1351
- Related ticket evidence: mika#1305 verdict trail (5 GROOMED cycles, zero body callouts)
