# Plan — fix(dispatch-lib): extend auto-rescue to commit-pushed-no-PR case (mika#1396)

## Problem

mika#1282's auto-rescue opens a draft PR ONLY when `RESCUED_DIRTY_WORKTREE=1` —
the pilot wrote files but never committed. The trigger condition misses a
related class:

- Pilot ran /ce:work, /ce:review, /ce:compound cleanly
- Pilot committed and pushed
- Pilot ran `gh pr create` — got transient AxiosError 5000ms timeout
- Pilot exited 1 after session
- Result: branch has commit on origin, but no PR exists

The existing mika#940 check (line ~860) detects this but classifies as
PIPELINE FAILURE rather than triggering rescue. Net effect: parent task
goes blocked; operator hand-opens the PR.

## Fix

Extend the auto-rescue trigger with a SIBLING condition:

```
RESCUED_PR_URL trigger fires when EITHER:
  - RESCUED_DIRTY_WORKTREE=1 (mika#1282 original case), OR
  - PR_URL empty AND branch has commits on origin AND $PRE_RUN_HEAD != $POST_RUN_HEAD
    (mika#1396 commit-pushed-no-PR case)
```

Same body template, same draft status. Distinguishes the two recovery
classes in the PR title and body for audit.

## Why draft PR (not normal)

mika#1282 uses draft because pilot didn't pass review. mika#1396 case is
different — pilot DID pass /ce:review. But:
1. Keeping recovery path uniform avoids two distinct PR shapes
2. Draft signals "operator visit required to verify pilot's review actually completed"
3. Operator marks ready once they confirm the work is genuinely complete

The audit-trail consistency (always-draft from auto-rescue) outweighs the
slight inconvenience of operator marking ready.

## AC

- AC1: When pilot exits non-zero AFTER committing+pushing (PRE_RUN_HEAD != POST_RUN_HEAD on origin, PR_URL empty), auto-rescue opens a draft PR.
- AC2: When pilot exits with RESCUED_DIRTY_WORKTREE=1 (mika#1282 original case), auto-rescue continues to fire as before.
- AC3: PR body distinguishes the two recovery classes for audit.
- AC4: Canonical `PR:` line emitted (mika#1352 contract) — already shipped today via mika#1440.
- AC5: dispatch-lib test suite (174 tests) continues to pass.

## Implementation

In `mika/skills/bundled/_shared/dispatch-lib.sh` around line 1979:

```bash
# Determine recovery class
RECOVERY_CLASS=""
if [ "${RESCUED_DIRTY_WORKTREE:-}" = "1" ]; then
    RECOVERY_CLASS="dirty-worktree"
elif [ -z "$PR_URL" ] && [ -n "$PRE_RUN_HEAD" ] && [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ] && [ "$SKILL" = "dev-pilot" ]; then
    RECOVERY_CLASS="commit-pushed-no-pr"
fi

if [ -n "$RECOVERY_CLASS" ] && [ -n "$REPO" ] && [ -n "$BRANCH" ] && [ -z "$PR_URL" ]; then
    # ... existing rescue PR creation, with class noted in title/body ...
fi
```

## Cross-repo port

Single-repo fix on mika's dispatch-lib. Deploys via `make deploy` to `~/.mika/skills/_shared/`.
