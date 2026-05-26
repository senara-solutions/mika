---
ticket: mika#1296
type: bug
title: "Auto-rescue silently swallows pre-commit hook failures"
created: 2026-05-26
---

# Plan: Fix auto-rescue silent pre-commit hook failures (mika#1296)

## Problem

Unit 1 of `_run_claude_pilot` in `dispatch-lib.sh` (mika#1282 auto-rescue) runs `git commit` with stderr redirected to fd 9 (trace) and does **not check the exit code**. When lefthook's `rust-fmt` pre-commit hook rejects unformatted Rust code, the commit silently fails but the rescue flow proceeds as if it succeeded — setting `RESCUED_DIRTY_WORKTREE=1`, pushing, and opening a draft PR that contains only the pre-existing groom commit (no implementation).

Evidence: mika#1292 dispatch produced PR #1295 with only `+195/-0` (plan doc), despite the pilot writing 7 files including Rust source changes.

## Root cause

Line 443 of `dispatch-lib.sh`:

```bash
git -C "$WORKTREE_DIR" commit -m "wip(...)..." 2>&9
```

The `2>&9` redirect suppresses the error output, and the absence of `if !` or `||` means the non-zero exit code is ignored. Control falls through to `RESCUED_DIRTY_WORKTREE=1` unconditionally.

## Scope

**Single file:** `skills/bundled/_shared/dispatch-lib.sh`, Unit 1 auto-rescue block (lines ~443–460).

No Rust code changes. No schema changes. No new dependencies.

## Implementation

### Unit 1: Check commit exit code and auto-fix rust-fmt failures

**Location:** `dispatch-lib.sh` lines 443–460 (inside the `if git -C "$WORKTREE_DIR" diff --cached --quiet` guard).

Replace the unconditional commit with a checked commit + retry pattern:

```bash
# Attempt rescue commit — capture stderr for hook-failure diagnosis
RESCUE_COMMIT_ERR="$WORKTREE_DIR/.iterate/rescue-commit-err"
mkdir -p "$WORKTREE_DIR/.iterate" 2>/dev/null || true

if git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): impl staged by post-flight recovery (mika#1282)

Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
Auto-rescued by dispatch-lib dirty-worktree detection.
Scaffold paths excluded (mika#1288)." 2>"$RESCUE_COMMIT_ERR"; then
    # Commit succeeded on first try — proceed normally
    :
elif grep -q "rust-fmt\|cargo fmt\|rustfmt" "$RESCUE_COMMIT_ERR" 2>/dev/null; then
    # Pre-commit rust-fmt hook rejected — auto-fix and retry
    echo "NOTE: rescue commit rejected by rust-fmt hook — running cargo fmt and retrying" >&2
    (cd "$WORKTREE_DIR" && cargo fmt --all 2>&9) || true
    git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' 2>&9

    if ! git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): impl staged by post-flight recovery (mika#1282)

Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
Auto-rescued by dispatch-lib dirty-worktree detection (cargo fmt applied).
Scaffold paths excluded (mika#1288)." 2>"$RESCUE_COMMIT_ERR"; then
        # Retry also failed — abort rescue, leave dirty
        RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -20)
        RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook after cargo-fmt retry.
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}

${RESULT}"
        # Do NOT set RESCUED_DIRTY_WORKTREE — prevents empty draft PR
    fi
else
    # Unknown hook failure — abort rescue, leave dirty
    RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -20)
    RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt).
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}

${RESULT}"
    # Do NOT set RESCUED_DIRTY_WORKTREE — prevents empty draft PR
fi
```

The key behavioral changes:

1. **Capture commit stderr** to a file (`rescue-commit-err`) instead of discarding to fd 9.
2. **Check exit code** — the `if git commit` pattern makes success/failure explicit.
3. **On rust-fmt failure:** run `cargo fmt --all`, re-stage (respecting mika#1288 exclusion), retry commit.
4. **On retry failure or unknown hook failure:** emit structured `PIPELINE FAILURE` marker, do NOT set `RESCUED_DIRTY_WORKTREE=1`, leave worktree dirty for operator inspection.
5. **Only set `RESCUED_DIRTY_WORKTREE=1`** after a confirmed successful commit (first try or retry).

### Unit 2: Guard RESCUED_DIRTY_WORKTREE behind commit success

Move the `RESCUED_DIRTY_WORKTREE=1` and `POST_RUN_HEAD` update **inside** the success path (after confirmed commit), not as a fallthrough. The current code at lines 449–460 sets these unconditionally — they must be conditional on the commit (or retry) succeeding.

The restructured block uses `if/elif/else` (Unit 1 above), so `RESCUED_DIRTY_WORKTREE=1` and `POST_RUN_HEAD` update only execute in the `if` (first-try success) and `elif` retry-success branches. The `else` and retry-failure branches skip them.

### Unit 3: Preserve existing RESULT/RESCUED_FILES logic

The `RESCUED_FILES` computation (line 441) stays where it is — before the commit attempt. It's used in the PIPELINE FAILURE message regardless of commit outcome. The RESULT message amendment (lines 452–457) moves inside the success path, since it should only say "auto-committed" when the commit actually succeeded.

## Control flow (after fix)

```
dirty worktree detected?
  └─ yes → stage files (excluding scaffold)
       └─ anything staged?
            └─ yes → compute RESCUED_FILES
                 └─ attempt git commit
                      ├─ success → set RESCUED_DIRTY_WORKTREE=1, update POST_RUN_HEAD, amend RESULT
                      ├─ rust-fmt hook failure → cargo fmt → re-stage → retry commit
                      │    ├─ retry success → set RESCUED_DIRTY_WORKTREE=1, update POST_RUN_HEAD, amend RESULT
                      │    └─ retry failure → PIPELINE FAILURE marker, leave dirty
                      └─ other hook failure → PIPELINE FAILURE marker, leave dirty
```

## Testing

1. **Manual verification:** Re-exercise on a ticket like mika#1292 (Rust changes that don't pass rustfmt). Confirm: `cargo fmt` runs, retry succeeds, PR contains real content.
2. **Edge cases to verify:**
   - Pilot writes only non-Rust files → commit succeeds on first try (no hook trigger) → existing behavior preserved.
   - Pilot writes Rust files that already pass fmt → commit succeeds on first try → no retry needed.
   - Pilot writes Rust files with clippy errors (not fmt) → falls into "unknown hook" branch → PIPELINE FAILURE, no empty PR.
   - `.iterate/` directory doesn't exist → `mkdir -p` handles it.
   - `cargo fmt` itself fails (e.g., syntax error in Rust code) → `|| true` absorbs it, retry commit still runs (may succeed if fmt wasn't the only issue, or fail with different hook error).

## Risk assessment

**Low risk.** Changes are confined to a single shell function block in dispatch-lib.sh. The fix adds conditional logic around an existing unconditional path — worst case regression is the rescue commit failing and leaving the worktree dirty (which is strictly better than the current behavior of opening an empty PR).

No Rust compilation, no schema migration, no API changes.
