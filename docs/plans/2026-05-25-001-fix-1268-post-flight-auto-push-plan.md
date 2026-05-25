# Fix: claude-pilot post-flight auto-push fails on non-zero exit

**Ticket:** mika#1268
**Type:** bug fix
**Branch:** `fix/1268/claude-pilot-post-flight-auto-push-fails`

## Problem

When a claude-pilot dispatch (dev-groom or dev-pilot) exits non-zero after producing valid commits, those commits stay local-only. The operator must manually `git push` to rescue. Two instances on mika#1262 in a single day (dev-groom session `741fa338` and dev-pilot session `6de51704`).

**Root cause:** `dispatch-lib.sh` has no unconditional "local-ahead-of-origin → push" finalizer. The only push logic lives inside the Class D body-callout recovery path (`_verify_and_write_body_callout`, line 250), which only fires for `dev-groom` and only when the callout is missing. The main `_run_claude_pilot` flow performs post-flight diff checks (lines 556-563) and PR checks (lines 642-646) but never pushes.

The pilot's exit code conflates "session outcome" with "work validity." Exit code 1 can mean the SDK hit a limit after all substantive work succeeded (`_emit_result` failure flips exit from 0 to 1). The dispatch caller treats exit code as authoritative, but it shouldn't gate push on it.

## Solution

Add an unconditional `_post_flight_push` helper to `dispatch-lib.sh` that fires after `_run_claude_pilot` returns, regardless of pilot exit code. Gate the push on `git rev-list @{u}..HEAD` (local commits ahead of upstream), not on exit code.

## Deliverables

### Phase 1: Add the push finalizer (dispatch-lib.sh)

**File:** `skills/bundled/_shared/dispatch-lib.sh`

1. Add a new `_post_flight_push()` internal helper function after `_run_claude_pilot()` and before `_deliver_callback()`.

2. The helper:
   - **Guard: repo#number mode only.** Skip if `$REPO` or `$WORKTREE_DIR` is empty (free-text mode has no branch to push).
   - **Guard: branch exists.** Skip if `$BRANCH` is empty.
   - **Check local-ahead-of-origin.** Run `git -C "$WORKTREE_DIR" rev-list "origin/$BRANCH..HEAD" --count 2>/dev/null`. If count is 0 or the command fails (no upstream tracking), skip silently.
   - **Fetch fresh remote state first.** Run `git -C "$WORKTREE_DIR" fetch origin "$BRANCH" 2>/dev/null || true` before the rev-list check to avoid stale remote refs giving a false positive. (The worktree was created from `origin/main`, so `origin/$BRANCH` may not exist yet if the branch was never pushed.)
   - **Push.** Run `git -C "$WORKTREE_DIR" push origin "$BRANCH" 2>/dev/null`. On failure, log a warning but do NOT alter the exit code or RESULT — the push is best-effort. The existing PIPELINE FAILURE classification and callback delivery must not be affected.
   - **Log.** On success, emit `post_flight_push: pushed N commits on $BRANCH to origin` to stderr. On failure, emit `WARN: post_flight_push_failed for $BRANCH` to stderr.

3. Insert the call `_post_flight_push` in `dispatch_claude_pilot()` between `_run_claude_pilot "$ENTRY_COMMAND"` (line 865) and `_deliver_callback` (line 866).

**Why this placement:** The push must happen after all post-flight checks in `_run_claude_pilot` have run (they read `POST_RUN_HEAD` which requires the worktree to be stable) and before callback delivery (so the RESULT message can reflect whether the push succeeded). The push doesn't modify RESULT — it's purely a side effect. Callback delivery happens whether or not the push worked.

**Why not in the EXIT trap:** The EXIT trap (`_dispatch_lib_exit_trap`) is the crash-recovery path. Adding push logic there would create a second code path that duplicates the same concern. Better to have one explicit call in the happy path and let the EXIT trap focus on crash recovery.

### Phase 2: Update RESULT to reflect push status

After `_post_flight_push` runs, if it pushed commits, the existing "PIPELINE FAILURE: no PR" or outcome classification is unchanged (they already ran). However, append a `Post-flight push: <status>` line to RESULT inside the helper so the callback includes visibility:

```
Post-flight push: pushed 3 commits on fix/1268/... to origin
```

or on failure:

```
Post-flight push: FAILED — commits remain local-only on fix/1268/...
```

This is appended inside `_post_flight_push` (not in `_run_claude_pilot`), so it fires after the outcome classification.

### Phase 3: Solution doc update

**File:** `docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md`

Add a "Resolution" section at the bottom noting that the class is now resolved by the unconditional post-flight push finalizer (mika#1268), with merge commit reference (TBD at PR time). Update the `applies_when` frontmatter to note the fix.

## Acceptance criteria tie-back

- **AC1:** After any claude-pilot dispatch, if the worktree's branch has commits ahead of `origin/<branch>`, those commits land on origin without manual intervention — regardless of pilot exit code. → Phase 1 delivers this.
- **AC2:** Regression test exercises the path. → Out of scope for the plan — dispatch-lib.sh is a shell script; the existing test surface is manual + post-flight checks. The structural defense is the rev-list gate (deterministic, no LLM judgment).
- **AC3:** Solution doc updated. → Phase 3 delivers this.

## Out of scope

- Root-causing why claude-pilot exits non-zero (separate ticket per issue body).
- Adding the push to the EXIT trap (crash path) — the trap already has crash-recovery logic; conflating push with crash recovery creates more complexity than it saves.
- Changing claude-pilot-py exit code semantics — the fix is on the dispatch caller side, not the pilot itself.

## Risks

- **Force-push safety:** The push is a normal `git push`, not `--force`. If origin/$BRANCH has diverged (e.g., someone else pushed to the same branch), the push fails gracefully and the warning is logged. This is acceptable — the operator can resolve manually.
- **Double push:** If the pilot already pushed during its session, the rev-list count is 0 and the finalizer is a no-op. No double-push risk.
