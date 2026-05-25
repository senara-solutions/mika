# Fix: claude-pilot post-flight auto-push fails on non-zero exit

**Ticket:** mika#1268
**Type:** bug fix
**Branch:** `fix/1268/claude-pilot-post-flight-auto-push-fails`
**Revision:** rev 2 (addresses architect first-pass F1 blocking + F2/F3 non-blocking)

## Problem

When a claude-pilot dispatch (dev-groom or dev-pilot) exits non-zero after producing valid commits, those commits stay local-only. The operator must manually `git push` to rescue. Two instances on mika#1262 in a single day (dev-groom session `741fa338` and dev-pilot session `6de51704`).

**Root cause:** `dispatch-lib.sh` has no unconditional "local-ahead-of-origin → push" finalizer. The only push logic lives inside the Class D body-callout recovery path (`_verify_and_write_body_callout`, line 250), which only fires for `dev-groom` and only when the callout is missing. The main `_run_claude_pilot` flow performs post-flight diff checks (lines 556-563) and PR checks (lines 642-646) but never pushes.

The pilot's exit code conflates "session outcome" with "work validity." Exit code 1 can mean the SDK hit a limit after all substantive work succeeded (`_emit_result` failure flips exit from 0 to 1). The dispatch caller treats exit code as authoritative, but it shouldn't gate push on it.

## Solution

Add an unconditional `_post_flight_push` helper to `dispatch-lib.sh` that fires after `_run_claude_pilot` returns, regardless of pilot exit code. The helper handles **both first-push and existing-remote cases** explicitly — the prior revision's `rev-list-fails-then-skip` semantics inverted the first-push case (the most important one), which the architect flagged as F1.

## Deliverables

### Phase 1: Add the push finalizer (dispatch-lib.sh)

**File:** `skills/bundled/_shared/dispatch-lib.sh`

1. Add a new `_post_flight_push()` internal helper function after `_run_claude_pilot()` and before `_deliver_callback()`.

2. The helper:
   - **Guard: repo#number mode only.** Skip if `$REPO`, `$WORKTREE_DIR`, or `$BRANCH` is empty (free-text mode has no branch to push).
   - **Fetch fresh remote state.** Run `git -C "$WORKTREE_DIR" fetch origin "$BRANCH" 2>/dev/null || true`. This refreshes `origin/$BRANCH` if it exists, or no-ops if it doesn't (first-push case). The `|| true` swallows the expected failure on first-push.
   - **Branch the logic on remote-ref existence (F1 fix):**
     ```
     if git -C "$WORKTREE_DIR" rev-parse --verify "origin/$BRANCH" >/dev/null 2>&1; then
         # Existing-remote case — push only if HEAD is ahead.
         ahead=$(git -C "$WORKTREE_DIR" rev-list "origin/$BRANCH..HEAD" --count 2>/dev/null || echo 0)
         [ "${ahead:-0}" -eq 0 ] && return 0
     fi
     # First-push case (no origin/$BRANCH ref) — always push.
     # Class D push at line 250, if it ran, already updated origin/$BRANCH; the
     # existing-remote branch above would catch that case with ahead=0 and return.
     ```
   - **Push with upstream tracking.** Run `git -C "$WORKTREE_DIR" push -u origin "$BRANCH" 2>/dev/null`. The `-u` flag sets upstream tracking on first push so subsequent operations have an upstream to compare against.
   - **On failure:** log warning, do NOT alter exit code or RESULT structure beyond the appended status line (see Phase 2). The existing PIPELINE FAILURE classification and callback delivery must not be affected.
   - **Log on success:** emit `post_flight_push: pushed $BRANCH to origin` to stderr.
   - **Log on failure:** emit `WARN: post_flight_push_failed for $BRANCH — commits remain local-only` to stderr (with the captured push error from stderr if available).

3. Insert the call `_post_flight_push` in `dispatch_claude_pilot()` between `_run_claude_pilot "$ENTRY_COMMAND"` (line 865) and `_deliver_callback` (line 866).

**Why this placement:** The push must happen after all post-flight checks in `_run_claude_pilot` have run (they read `POST_RUN_HEAD` which requires the worktree to be stable) and before callback delivery (so RESULT can reflect whether the push succeeded). The push doesn't modify RESULT structure — it only appends one line.

**Why not in the EXIT trap:** The EXIT trap (`_dispatch_lib_exit_trap`) is the crash-recovery path. Adding push logic there would duplicate concerns. One explicit call in the happy path + EXIT trap focused on crash recovery is cleaner.

### Phase 2: Append push status line to RESULT (F3-safe)

After `_post_flight_push` runs, append exactly one line to RESULT following the existing line-based convention (matches the `STATUS=` / `PIPELINE FAILURE:` / `Post-condition:` pattern used elsewhere in the file):

```
Post-flight push: pushed to origin/$BRANCH
```

or on failure:

```
Post-flight push: FAILED — commits remain local-only on $BRANCH
```

**F3 safety constraints:**
- One line, ASCII, no embedded newlines beyond the leading `\n` from `${RESULT}\n...`.
- Append happens inside `_post_flight_push`, AFTER outcome classification (PIPELINE FAILURE / STATUS=...) has already populated RESULT. The 92K cap at line 88 (`RESULT=$(printf '%s' "$RESULT" | head -c 92000)`) is not approached by adding ~80 bytes.
- The line does not collide with any existing parser regex in `dispatch-lib.sh` or downstream callback handlers (`self-dev-callback`).

### Phase 3: F2 — interaction with Class D push at line 250

The architect flagged F2 (non-blocking): the existing Class D body-callout recovery already pushes at line 250 when it recovers a missing callout. The new finalizer would double-push in that path.

**Resolution: the finalizer is idempotent by construction.** Class D's push at line 250 succeeds → origin/$BRANCH advances to match local HEAD. The finalizer's fetch then refreshes the local view of origin/$BRANCH. The rev-list ahead-count returns 0. The finalizer returns without pushing.

Concretely the sequence is:
1. `_verify_and_write_body_callout` (Class D) pushes if it had to commit a body-callout recovery.
2. `_post_flight_push` fetches origin/$BRANCH → updated to match Class D's push.
3. Rev-list ahead-count = 0 → finalizer skips.

This works without additional flags or state. The plan does NOT add a "Class D already pushed" sentinel because the fetch-then-check pattern handles it implicitly.

### Phase 4: Solution doc update

**File:** `docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md`

Add a "Resolution" section at the bottom noting the class is resolved by the unconditional post-flight push finalizer (mika#1268), with merge commit reference (TBD at PR time). Update the `applies_when` frontmatter to note the fix.

## Acceptance criteria tie-back

- **AC1:** After any claude-pilot dispatch, if the worktree's branch has commits ahead of `origin/<branch>` — OR the remote branch doesn't yet exist at all (first push) — those commits land on origin without manual intervention, regardless of pilot exit code. → Phase 1 delivers this.
- **AC2:** Regression test exercises the path. → Out of scope for the plan — dispatch-lib.sh is a shell script; existing test surface is manual + post-flight checks. The structural defense is the explicit branch on `rev-parse --verify origin/$BRANCH` (deterministic, no LLM judgment).
- **AC3:** Solution doc updated. → Phase 4 delivers this.

## Out of scope

- Root-causing why claude-pilot exits non-zero (separate ticket per issue body).
- Adding push to the EXIT trap (crash path) — the trap already has crash-recovery logic; conflating concerns creates more complexity than it saves.
- Changing claude-pilot-py exit code semantics — fix is on dispatch caller side, not the pilot itself.

## Risks

- **Force-push safety:** The push is a normal `git push -u`, not `--force`. If origin/$BRANCH has diverged (e.g., concurrent push to same branch), the push fails gracefully and the warning is logged. The operator can resolve manually.
- **Double push:** Addressed in Phase 3 — Class D's push is reflected by the fetch-then-check sequence; finalizer no-ops when origin is already in sync.
- **First-push without commits:** If `$BRANCH` is freshly created from `origin/main` and no commits were made, the push is a no-op that creates an empty branch on origin. Acceptable — the branch was deliberately checked out, so its existence on origin is fine. Subsequent CI will not run because no PR exists.

## Grooming history

- /ce:plan → mika-arch first-pass: ITERATE (F1 blocking, F2/F3 non-blocking)
- Plan revised (rev 2): F1 addressed by explicit first-push branch in Phase 1; F2 addressed by Phase 3 (fetch-then-check idempotency); F3 addressed by Phase 2 (line-based convention + 92K cap headroom)
- → mika-arch second-pass: pending
