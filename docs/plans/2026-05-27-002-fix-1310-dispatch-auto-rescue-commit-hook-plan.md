# Plan: fix(dispatch): auto-rescue commit hook-rejection false positive recurs (mika#1310)

## Problem

Every dev-pilot dispatch that exits dirty triggers `PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt). Hook output:` with an **empty** hook output field. Operator-direct `git commit` on the identical staged set succeeds cleanly. This defeats the mika#1282 auto-rescue feature entirely.

## Root Cause

**The stderr redirect path is invalid in git worktrees.**

Line 458 of `dispatch-lib.sh`:
```bash
RESCUE_COMMIT_ERR="$WORKTREE_DIR/.git/mika-rescue-commit-err"
```

In git worktrees, `.git` is a **file** (containing `gitdir: <real-path>`), not a directory. The redirect `2>"$RESCUE_COMMIT_ERR"` fails with "Not a directory" before `git commit` ever executes. Bash sees non-zero exit, the error file doesn't exist, `grep` and `cat` on it produce nothing, and the code falls through to the "unknown hook failure" branch with empty output.

**Proof:** `file $WORKTREE_DIR/.git` returns `ASCII text`; `touch $WORKTREE_DIR/.git/anything` fails with `Not a directory`.

The comment at line 456 ("Use a worktree-local scratch file under .git/") reveals the assumption: the author expected `.git` to be a directory (true in normal repos, false in worktrees — and auto-rescue ONLY runs in worktrees).

## Fix

### Unit 1: Fix the stderr capture path (the actual bug)

**File:** `skills/bundled/_shared/dispatch-lib.sh`

Replace the hardcoded `.git/` path with `git rev-parse --git-dir`, which correctly resolves the actual git directory in both regular repos and worktrees:

```bash
# Before (line 458):
RESCUE_COMMIT_ERR="$WORKTREE_DIR/.git/mika-rescue-commit-err"

# After:
RESCUE_COMMIT_ERR="$(git -C "$WORKTREE_DIR" rev-parse --git-dir)/mika-rescue-commit-err"
```

This pattern is already used at line 261 of the same file (`git -C "$WORKTREE_DIR" rev-parse --git-dir`), so it's a known-good idiom in this codebase.

**Update the comment** at lines 456-457 to note worktree awareness:

```bash
# Attempt rescue commit — capture stderr for hook-failure diagnosis (mika#1296).
# Use git rev-parse --git-dir (not $WORKTREE_DIR/.git/) because .git is a
# file in worktrees, not a directory (mika#1310).
```

### Unit 2: Improve failure classification (defense in depth)

The current code assumes any `git commit` failure = pre-commit hook rejection. This is fragile — `git commit` can fail for many reasons (GPG signing, lock files, empty message, etc.). Add explicit hook-failure detection:

**a) Capture both stdout and stderr.** Lefthook writes its formatted output to stdout, not stderr. The current `2>` redirect misses all hook output even when hooks genuinely fail:

```bash
# Before:
if git -C "$WORKTREE_DIR" commit -m "..." 2>"$RESCUE_COMMIT_ERR"; then

# After:
if git -C "$WORKTREE_DIR" commit -m "..." >"$RESCUE_COMMIT_OUT" 2>"$RESCUE_COMMIT_ERR"; then
```

Add `RESCUE_COMMIT_OUT` alongside `RESCUE_COMMIT_ERR` (same directory, different suffix).

**b) Distinguish hook failure from other git-commit failures.** After the commit fails, check for hook-specific signals before assuming hook rejection:

```bash
# After commit fails, combine stdout+stderr for full diagnostic
RESCUE_COMBINED=$(cat "$RESCUE_COMMIT_OUT" "$RESCUE_COMMIT_ERR" 2>/dev/null | head -40)

# Check for hook-failure signals in combined output
if echo "$RESCUE_COMBINED" | grep -qi "rust-fmt\|cargo fmt\|rustfmt"; then
    # rust-fmt hook path (existing logic)
elif echo "$RESCUE_COMBINED" | grep -qi "lefthook\|pre-commit\|hook"; then
    # Generic hook failure — surface combined output
    RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt).
Hook output: ${RESCUE_COMBINED}
..."
else
    # Non-hook failure — different message for accurate diagnosis
    RESULT="PIPELINE FAILURE: auto-rescue commit failed (not a hook rejection).
git output: ${RESCUE_COMBINED}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}
..."
fi
```

This prevents future misdiagnosis when `git commit` fails for non-hook reasons.

**c) Clean up both files** in all exit paths (success, hook failure, non-hook failure):

```bash
rm -f "$RESCUE_COMMIT_ERR" "$RESCUE_COMMIT_OUT"
```

### Unit 3: Update the rust-fmt retry path

The rust-fmt `grep` at line 480 currently only checks stderr. Update it to check the combined output (same pattern as Unit 2b). Also update the retry commit at line 490 to use the same `rev-parse --git-dir` path and stdout+stderr capture.

## Files Changed

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Fix stderr path, capture stdout, improve failure classification |

## Testing

1. **Manual verification:** Run a dev-pilot dispatch on a ticket with real file changes. The auto-rescue path should now succeed (commit + push + draft PR) instead of false-positive failing.
2. **Simulated hook failure:** Temporarily break `lefthook.yml` (e.g., add a command that always fails), verify that the PIPELINE FAILURE message now carries actual hook output.
3. **Simulated non-hook failure:** Temporarily make `git commit` fail for a non-hook reason (e.g., corrupt index), verify the message says "not a hook rejection" instead of "rejected by pre-commit hook".

## Risk Assessment

**Low risk.** The change is confined to the auto-rescue error path in `dispatch-lib.sh`. The happy path (pilot commits normally) is unaffected. The fix uses an existing pattern (`rev-parse --git-dir`) already proven in the same file.

## Sequence

Unit 1 first (unblocks all auto-rescue), then Unit 2 + Unit 3 (defense in depth). All three can ship in a single commit since they touch adjacent lines in the same function.
