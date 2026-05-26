# Plan: Pre-push duplicate-commit guard in dispatch-lib (#784)

## Problem

mika#747 landed a rebase-or-abort guard at claude-pilot handler startup, closing the stale-base gap at session open. But mid-session `git pull` or `git merge main` can reintroduce duplicate-hash copies of upstream commits onto the branch, producing `mergeable=CONFLICTING` PRs on GitHub even though content is identical.

**Observed failure (mika#286 / PR #782):** After the #747 rebase guard ran cleanly at startup, the claude-pilot session ran `git pull` / `git pull origin main` mid-session. This created commit `8693b3fd` — a duplicate of main's `14279524` (same author date, message, diff, different hash). GitHub's 3-way merge saw both commits touching the same lines → CONFLICTING.

**Root cause:** Two gaps in #747:
1. Rebase-or-abort runs once at session START — no end-of-session sanity check
2. No structural guard on what git operations the session runs mid-flight

## Decision: Option A — Pre-push duplicate-commit detection

The ticket presents three options. **Option A (pre-push rebase check) is the right call.** Rationale:

- **Covers the observed failure class.** The bug manifests at push time — a guard there catches it regardless of how the duplicate arrived (pull, merge, cherry-pick, or future unknown paths).
- **Simpler than restricting session ops (Option B).** Permission-policy already omits `git pull` from TIER 1 auto-approve, but the observed failure proves prompt-level classification isn't airtight. Structural session restrictions would require either a git hook in the worktree or a wrapper script replacing `git` — both are fragile and introduce new failure modes.
- **Belt-and-suspenders (Option C) is premature.** Option A alone closes the gap. If a future failure mode shows pull-restriction is also needed, it can be added as a follow-up.

## Implementation

### Step 1: Add `_check_duplicate_commits()` function to dispatch-lib.sh

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** After `_push_branch()` (line ~693), before the iterate-loop primitives section.

```bash
_check_duplicate_commits() {
    # Pre-push guard: detect commits on the branch that are patch-equivalent
    # to commits already on origin/main. These duplicates cause
    # mergeable=CONFLICTING on GitHub even though content is identical.
    # See mika#784 for the observed failure mode.
    #
    # Uses git log --cherry-pick --right-only which shows commits on HEAD
    # that do NOT have a patch-equivalent on origin/main. By inverting
    # (--left-right --cherry-mark), we can detect commits marked as '='
    # (equivalent on both sides).
    
    [ -n "$WORKTREE_DIR" ] || return 0
    
    # Fetch fresh main to compare against
    git -C "$WORKTREE_DIR" fetch origin main 2>/dev/null || return 0
    
    # Find commits on HEAD that are patch-equivalent to commits on origin/main.
    # --cherry-mark marks equivalent commits with '=' prefix.
    # --right-only shows only commits on the right side (HEAD).
    # Equivalent commits on HEAD = duplicates that will conflict.
    local duplicates
    duplicates=$(git -C "$WORKTREE_DIR" log --cherry-mark --right-only --no-walk \
        --format="%m %H %s" origin/main...HEAD 2>/dev/null \
        | grep "^=" || true)
    
    [ -z "$duplicates" ] && return 0
    
    # Duplicates found — attempt automatic rebase to clean them up
    echo "WARN: duplicate-commit guard found patch-equivalent commits on branch:" >&2
    echo "$duplicates" >&2
    echo "Attempting rebase onto origin/main to deduplicate..." >&2
    
    if git -C "$WORKTREE_DIR" rebase origin/main 2>/dev/null; then
        echo "Rebase succeeded — duplicate commits resolved." >&2
        return 0
    fi
    
    # Rebase failed — abort and report
    git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
    echo "ERROR: duplicate-commit rebase failed. Branch has commits equivalent to main:" >&2
    echo "$duplicates" >&2
    return 1
}
```

**Design notes:**
- Uses `git log --cherry-mark` which compares patches (not hashes) — detects duplicate content regardless of hash divergence.
- Attempts automatic rebase as self-heal (same pattern as startup guard). Rebase naturally drops patch-equivalent commits when replaying onto a base that already has them.
- On rebase failure, returns non-zero so `_push_branch` can surface the error.
- Fetches `origin/main` fresh to compare against current remote state, not stale local refs.

### Step 2: Wire `_check_duplicate_commits()` into `_push_branch()`

**File:** `skills/bundled/_shared/dispatch-lib.sh`
**Location:** Inside `_push_branch()`, after the mode guard (line 663) and before the fetch/push logic (line 666).

Insert the call before the existing push logic:

```bash
_push_branch() {
    [ -n "$REPO" ] && [ -n "$WORKTREE_DIR" ] && [ -n "$BRANCH" ] || return 0

    # Pre-push duplicate-commit guard (mika#784)
    if ! _check_duplicate_commits; then
        echo "WARN: push_branch skipped — duplicate-commit guard failed for $BRANCH" >&2
        RESULT="${RESULT}
Push: SKIPPED — duplicate-commit guard detected patch-equivalent commits on branch that could not be auto-rebased. Manual resolution required."
        return 1
    fi

    # (existing fetch + push logic follows unchanged)
    git -C "$WORKTREE_DIR" fetch origin "$BRANCH" 2>/dev/null || true
    ...
```

**Why before push, not after:** The guard must run before `git push` to prevent the CONFLICTING state from reaching GitHub. Post-push detection would require force-push to fix, which is a more dangerous operation.

### Step 3: Retire the tactical Git discipline prompt section

**File:** `.claude/commands/mika.md`
**Action:** Remove the "Git discipline (MANDATORY)" section added in commit `58b64a87`.

The structural pre-push guard makes the prompt-level rule fully redundant:
- The prompt rule forbids `git pull` / `git merge main` during the pipeline.
- The structural guard catches and auto-fixes the consequences (duplicate commits) regardless of what git commands ran.
- Keeping the prompt rule after the structural fix is dead weight that increases prompt token count without adding safety.

**Important:** The permission-policy already omits `git pull` from TIER 1 auto-approve. Between the structural guard (catches duplicates) and permission-policy (escalates `git pull` to TIER 3), the tactical prompt section is doubly redundant.

### Step 4: Update the #747 solution doc

**File:** `docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md`
**Action:** Add a "Follow-up: mid-session duplicate-commit guard (#784)" section documenting:
- The gap in #747's startup-only guard
- The observed failure in PR #782
- The fix: pre-push `--cherry-mark` detection + auto-rebase in `_push_branch()`
- Cross-reference to this ticket

### Step 5: Compound the solution

**File:** `docs/solutions/logic-errors/mid-session-duplicate-commit-pre-push-guard-2026-05-26.md`
**Action:** Create a standalone solution doc for #784 covering:
- Problem: mid-session git pull creates duplicate-hash commits
- Root cause: #747 guard is startup-only
- Solution: `_check_duplicate_commits()` pre-push guard using `git log --cherry-mark`
- Tags: `module: dispatch-lib`, `problem_type: logic-error`, `tags: [git, claude-pilot, duplicate-commit, cherry-mark]`

## Files changed

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Add `_check_duplicate_commits()` + wire into `_push_branch()` |
| `.claude/commands/mika.md` | Remove "Git discipline (MANDATORY)" tactical section |
| `docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md` | Add follow-up section |
| `docs/solutions/logic-errors/mid-session-duplicate-commit-pre-push-guard-2026-05-26.md` | New solution doc |

## Testing

1. **Unit test the cherry-mark detection:** Create a test script that:
   - Inits a repo with a commit on main
   - Creates a branch, cherry-picks the same commit (producing a duplicate-hash scenario)
   - Runs `_check_duplicate_commits` and verifies it detects the duplicate
   - Verifies the auto-rebase resolves it

2. **Regression test:** Verify the existing `_rebase_or_abort` startup guard still works unchanged.

3. **Manual end-to-end:** Dispatch a dev-pilot session, verify `_push_branch` logs show the duplicate-commit check running (even when no duplicates exist — clean no-op path).

## Risk assessment

- **Low risk.** The guard is additive — it runs before push and falls back to skip-push-with-warning on failure. No existing behavior is changed; only a new check is inserted.
- **Auto-rebase failure mode:** If the auto-rebase fails (due to real conflicts, not just duplicates), the guard returns non-zero and push is skipped. The PR will not be created in CONFLICTING state. The pilot session exits with a push failure, which surfaces via dispatch-lib's existing error propagation.
- **Performance:** One additional `git fetch origin main` + `git log --cherry-mark` per push. Both are fast operations on typical branch sizes (<100 commits).
