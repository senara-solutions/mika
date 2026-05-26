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
# Attempt rescue commit — capture stderr for hook-failure diagnosis.
# Use a worktree-local scratch file under .git/ to avoid coupling with
# .iterate/ (the iterate-loop artifact directory — review-guide.md § Orthogonality).
RESCUE_COMMIT_ERR="$WORKTREE_DIR/.git/mika-rescue-commit-err"

if git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): impl staged by post-flight recovery (mika#1282)

Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
Auto-rescued by dispatch-lib dirty-worktree detection.
Scaffold paths excluded (mika#1288)." 2>"$RESCUE_COMMIT_ERR"; then
    # Commit succeeded on first try — proceed normally
    rm -f "$RESCUE_COMMIT_ERR"
elif grep -q "rust-fmt\|cargo fmt\|rustfmt" "$RESCUE_COMMIT_ERR" 2>/dev/null; then
    # Pre-commit rust-fmt hook rejected — auto-fix and retry.
    # Capture cargo fmt stderr so it surfaces in the PIPELINE FAILURE message
    # if the retry also fails (review-guide.md § Single Responsibility — failure
    # paths must surface all available diagnostic information).
    CARGO_FMT_ERR=""
    echo "NOTE: rescue commit rejected by rust-fmt hook — running cargo fmt and retrying" >&2
    CARGO_FMT_ERR=$( (cd "$WORKTREE_DIR" && cargo fmt --all) 2>&1 ) || true
    git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' 2>&9

    if ! git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): impl staged by post-flight recovery (mika#1282)

Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
Auto-rescued by dispatch-lib dirty-worktree detection (cargo fmt applied).
Scaffold paths excluded (mika#1288)." 2>"$RESCUE_COMMIT_ERR"; then
        # Retry also failed — abort rescue, leave dirty.
        # Surface the full diagnostic chain: cargo fmt output + retry commit
        # hook output, so the operator can diagnose from the message alone
        # (mika#1296 acceptance criteria).
        RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -20)
        RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook after cargo-fmt retry.
cargo fmt stderr: ${CARGO_FMT_ERR:-<empty>}
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}

${RESULT}"
        # Do NOT set RESCUED_DIRTY_WORKTREE — prevents empty draft PR
    fi
    rm -f "$RESCUE_COMMIT_ERR"
else
    # Unknown hook failure — abort rescue, leave dirty
    RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -20)
    RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt).
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}

${RESULT}"
    # Do NOT set RESCUED_DIRTY_WORKTREE — prevents empty draft PR
    rm -f "$RESCUE_COMMIT_ERR"
fi
```

The key behavioral changes:

1. **Capture commit stderr** to a worktree-local scratch file (`$WORKTREE_DIR/.git/mika-rescue-commit-err`) instead of discarding to fd 9. The `.git/` location is worktree-local, never committed, and clearly scratch — avoiding coupling with `.iterate/` (the iterate-loop artifact directory per review-guide.md § Orthogonality).
2. **Check exit code** — the `if git commit` pattern makes success/failure explicit.
3. **On rust-fmt failure:** run `cargo fmt --all` with stderr captured to a variable (`CARGO_FMT_ERR`), re-stage (respecting mika#1288 exclusion), retry commit. If the retry also fails, the PIPELINE FAILURE message includes both `cargo fmt` stderr and the retry commit's hook output — giving the operator the full diagnostic chain (review-guide.md § Single Responsibility; mika#1296 acceptance criteria).
4. **On retry failure or unknown hook failure:** emit structured `PIPELINE FAILURE` marker, do NOT set `RESCUED_DIRTY_WORKTREE=1`, leave worktree dirty for operator inspection.
5. **Only set `RESCUED_DIRTY_WORKTREE=1`** after a confirmed successful commit (first try or retry).
6. **Cleanup:** `rm -f "$RESCUE_COMMIT_ERR"` on every exit path (success, retry-success, retry-failure, unknown-failure) — transient scratch file never lingers.

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

### Unit 4: Automated test for RESCUED_DIRTY_WORKTREE invariant on hook failure

**Location:** `skills/bundled/_shared/test-dispatch-lib.sh` — append a new test section following the established patterns (structural assertions + live git-repo exercises).

This test locks in the primary correctness invariant: *on pre-commit hook rejection, `RESCUED_DIRTY_WORKTREE` is NOT set* (mika#1288 established the pattern of adding `test-dispatch-lib.sh` tests for rescue-block behavioral invariants).

**Test A — structural assertion (code-shape):** Verify that in the rescue block of `dispatch-lib.sh`, `RESCUED_DIRTY_WORKTREE=1` appears only inside the commit-success path, not as a fallthrough after the `if git commit` block. Use `sed`/`grep` on the source, matching the pattern of Test C in the mika#1288 section.

**Test B — live invariant (git-repo exercise):** Create a temporary git repo with:
- A pre-commit hook that exits non-zero with "rust-fmt" in stderr (simulating lefthook rejection)
- A stub `cargo fmt` on PATH that succeeds (exits 0)
- A dirty tracked Rust file

Source the rescue block logic (or exercise it structurally via the function), then assert:
- `RESCUED_DIRTY_WORKTREE` is `0` (or unset) after the rescue block runs — the hook rejection prevented it from being set
- The `RESULT` variable contains `PIPELINE FAILURE` — confirming the failure path was taken
- The `RESULT` variable contains `cargo fmt stderr:` — confirming cargo fmt diagnostic surfacing (F1 fix)
- The scratch file (`$WORKTREE_DIR/.git/mika-rescue-commit-err`) does not exist after the block — confirming cleanup (F2 fix)

**Test C — structural assertion (scratch file location):** Verify the rescue block uses `$WORKTREE_DIR/.git/mika-rescue-commit-err` (not `.iterate/rescue-commit-err`) via grep on the source. This locks in the F2 fix against regression.

### Manual verification

1. Re-exercise on a ticket like mika#1292 (Rust changes that don't pass rustfmt). Confirm: `cargo fmt` runs, retry succeeds, PR contains real content.
2. **Edge cases to verify:**
   - Pilot writes only non-Rust files → commit succeeds on first try (no hook trigger) → existing behavior preserved.
   - Pilot writes Rust files that already pass fmt → commit succeeds on first try → no retry needed.
   - Pilot writes Rust files with clippy errors (not fmt) → falls into "unknown hook" branch → PIPELINE FAILURE, no empty PR.
   - `cargo fmt` itself fails (e.g., syntax error in Rust code) → `|| true` absorbs it, `CARGO_FMT_ERR` captures the diagnostic, retry commit still runs (may succeed if fmt wasn't the only issue, or fail with different hook error — either way the diagnostic is surfaced in the PIPELINE FAILURE message).

## Risk assessment

**Low risk.** Changes are confined to a single shell function block in dispatch-lib.sh. The fix adds conditional logic around an existing unconditional path — worst case regression is the rescue commit failing and leaving the worktree dirty (which is strictly better than the current behavior of opening an empty PR).

No Rust compilation, no schema migration, no API changes.

## Revision history

- rev 2 (2026-05-26): addressed F1 by capturing `cargo fmt` stderr to a variable (`CARGO_FMT_ERR`) and including it in the PIPELINE FAILURE message's diagnostic chain (review-guide.md § Single Responsibility — failure paths must surface all available diagnostic information); addressed F2 by relocating scratch file from `.iterate/rescue-commit-err` to `$WORKTREE_DIR/.git/mika-rescue-commit-err` with `rm -f` cleanup on all exit paths (review-guide.md § Orthogonality — .iterate/ is the iterate-loop artifact directory, not general scratch); addressed F3 by adding Unit 4 (automated test in `test-dispatch-lib.sh`) with structural assertions + live git-repo exercise verifying the `RESCUED_DIRTY_WORKTREE=0` invariant on hook failure, cargo fmt diagnostic surfacing, and scratch file cleanup (mika#1288 established the test pattern for rescue-block behavioral invariants).
