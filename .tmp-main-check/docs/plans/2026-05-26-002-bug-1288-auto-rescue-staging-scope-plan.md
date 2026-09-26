# Plan: bug(skill) mika#1288 — auto-rescue `git add -A` stages worktree scaffold files

## Problem

The mika#1282 auto-rescue code at `dispatch-lib.sh` line ~425 uses `git add -A` which stages ALL untracked files — including `.claude/commands/*.md` files that `_set_up_worktree` copies into the worktree as scaffold for the autonomous pilot. These are worktree-local artifacts, not pilot-authored work.

Evidence: PR #1287 included 18 extraneous `.claude/commands/*.md` files from the mika-platform workspace that don't belong in the mika repo.

## Root cause

`git add -A` stages both tracked-file modifications AND untracked files. The `.claude/commands/` snapshot is untracked (copied in after worktree creation from mika-platform), so it gets swept into the rescue commit.

## Solution

**Option 2 from the ticket: pathspec exclusion.** Replace:

```bash
git -C "$WORKTREE_DIR" add -A 2>&9
```

With:

```bash
git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' 2>&9
```

This stages everything dirty EXCEPT the worktree-scaffold directory. It's a single-line fix that:
- Preserves rescue of pilot-authored NEW files (source, plans, tests)
- Excludes the known scaffold path (`.claude/commands/`)
- Is readable and self-documenting via the pathspec syntax
- Doesn't require maintaining an external exclude list

### Why not `git add -u`?

`git add -u` only stages modifications to already-tracked files. This would miss legitimately NEW files authored by the pilot (e.g., a new source file, a new plan doc). The whole point of auto-rescue is to save work that didn't get committed — often that work IS new files.

### Why not a broader exclusion list?

Start minimal. The only confirmed scaffold path is `.claude/commands/`. If other scaffold patterns surface in future (`.claude/worktrees/`, etc.), extend the exclusion. YAGNI.

## Implementation

### Unit 1: Fix staging pathspec + empty-index guard (dispatch-lib.sh)

**File:** `skills/bundled/_shared/dispatch-lib.sh`  
**Location:** Line ~425 (inside the mika#1282 dirty-worktree rescue block)

#### 1a. Replace `git add -A` with pathspec exclusion

Change:
```bash
git -C "$WORKTREE_DIR" add -A 2>&9
```

To:
```bash
# Stage all dirty files EXCEPT worktree-scaffold paths copied by
# _set_up_worktree (mika#1288). .claude/commands/ contains slash-command
# snapshots from mika-platform — not pilot-authored content.
git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' 2>&9
```

#### 1b. Guard against empty index after exclusion (F1)

After the pathspec-filtered `git add`, the index may be empty if the dirty worktree contained ONLY scaffold files. Attempting `git commit` on an empty index fails silently (suppressed by `2>&9`), but `RESCUED_DIRTY_WORKTREE=1` would still be set — a false signal that causes Unit 2 (PR creation) to run against a branch with no rescue commit.

**Insert immediately after the `git add` line:**

```bash
# Guard: if pathspec exclusion left nothing staged, skip the rescue commit.
# This handles the edge case where the pilot wrote ONLY to scaffold paths.
# (review-guide.md § KISS — handle the empty-index case explicitly rather
# than silently producing a broken state)
if git -C "$WORKTREE_DIR" diff --cached --quiet 2>&9; then
    _pipeline_note "dirty worktree contained only scaffold paths (.claude/commands/) — no pilot content to rescue"
    RESCUED_DIRTY_WORKTREE=0
else
    # Compute accurate rescued-files list for the PIPELINE FAILURE message (F2).
    # DIRTY_FILES (from git status --porcelain) includes excluded scaffold paths;
    # RESCUED_FILES reflects what was actually staged and will be committed.
    # (review-guide.md § Single Responsibility — detection variable serves a
    # different purpose than the rescue-content log)
    RESCUED_FILES=$(git -C "$WORKTREE_DIR" diff --cached --name-only 2>&9)

    git -C "$WORKTREE_DIR" commit -m "wip() rescue uncommitted pilot work

Auto-rescued by dispatch-lib dirty-worktree recovery (mika#1282).
Scaffold paths excluded (mika#1288)." 2>&9
    # ... existing push + PR logic ...
    RESCUED_DIRTY_WORKTREE=1
fi
```

#### 1c. Use `RESCUED_FILES` in the PIPELINE FAILURE message (F2)

In the PIPELINE FAILURE output block that currently references `DIRTY_FILES`, replace:

```bash
"Files rescued:\n${DIRTY_FILES}"
```

With:

```bash
"Files rescued:\n${RESCUED_FILES}"
```

This ensures the operator-facing message accurately reflects what was committed, not what was dirty. `DIRTY_FILES` remains available for diagnostic logging but is not surfaced as "rescued" content.

### Unit 2: Add test case (test-dispatch-lib.sh)

**File:** `skills/bundled/_shared/test-dispatch-lib.sh`

Add a test function that verifies the staging exclusion and empty-index guard:

```bash
test_auto_rescue_excludes_scaffold_files() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    # Setup: create a git repo simulating a dirty worktree
    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # 1. Tracked file with a pilot modification
    echo "original" > "$test_dir/tracked.rs"
    git -C "$test_dir" add tracked.rs
    git -C "$test_dir" commit -m "add tracked" -q
    echo "modified by pilot" > "$test_dir/tracked.rs"

    # 2. Scaffold file (untracked) — must NOT be staged
    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold" > "$test_dir/.claude/commands/mika-groom-ticket.md"

    # 3. Pilot-authored new file (untracked) — must be staged
    mkdir -p "$test_dir/src"
    echo "fn main() {}" > "$test_dir/src/new_feature.rs"

    # Exercise: run the pathspec-filtered git add
    git -C "$test_dir" add -A -- ':!.claude/commands/' 2>/dev/null

    # Assert: check what was staged
    local staged
    staged=$(git -C "$test_dir" diff --cached --name-only)

    # Scaffold file must NOT appear in staged files
    if echo "$staged" | grep -q '.claude/commands/'; then
        echo "FAIL: scaffold file was staged"
        return 1
    fi

    # Pilot-authored new file must be staged
    if ! echo "$staged" | grep -q 'src/new_feature.rs'; then
        echo "FAIL: pilot-authored new file was not staged"
        return 1
    fi

    # Modified tracked file must be staged
    if ! echo "$staged" | grep -q 'tracked.rs'; then
        echo "FAIL: modified tracked file was not staged"
        return 1
    fi

    echo "PASS: scaffold excluded, pilot content staged"
}

test_auto_rescue_empty_index_guard() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    # Setup: repo where the ONLY dirty content is scaffold files
    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold only" > "$test_dir/.claude/commands/mika-groom-ticket.md"
    echo "# another scaffold" > "$test_dir/.claude/commands/mika.md"

    # Exercise: pathspec-filtered git add
    git -C "$test_dir" add -A -- ':!.claude/commands/' 2>/dev/null

    # Assert: index should be empty (diff --cached --quiet exits 0)
    if ! git -C "$test_dir" diff --cached --quiet 2>/dev/null; then
        echo "FAIL: index is not empty despite only scaffold files being dirty"
        return 1
    fi

    echo "PASS: empty index correctly detected for scaffold-only worktree"
}
```

Two test functions covering the two scenarios: (1) mixed content where scaffold is excluded and pilot content is staged, (2) scaffold-only dirty worktree where the empty-index guard should fire.

## Acceptance criteria mapping

| Criterion | How addressed |
|-----------|--------------|
| Auto-rescue does NOT include `.claude/commands/*.md` | Pathspec `:!.claude/commands/` excludes the directory (Unit 1a) |
| Auto-rescue DOES include pilot-authored new files | `git add -A` still stages all other untracked files (Unit 1a) |
| Scaffold-only worktree does not produce false `RESCUED_DIRTY_WORKTREE=1` | Empty-index guard skips commit + clears flag (Unit 1b) |
| PIPELINE FAILURE message accurately reflects rescued content | `RESCUED_FILES` from `git diff --cached --name-only` replaces `DIRTY_FILES` (Unit 1c) |
| Test verifies exclusion | `test_auto_rescue_excludes_scaffold_files` (Unit 2) |
| Test verifies empty-index guard | `test_auto_rescue_empty_index_guard` (Unit 2) |
| Re-exercise mika#897-style ticket | Manual verification (operator re-runs on a test ticket) |

## Risk assessment

**Low risk.** Single-line change to a pathspec in a rescue path that only fires on zero-commit dev-pilot exits with dirty worktrees. The pathspec syntax is well-documented git behavior. Worst case if the exclusion doesn't work: same behavior as before (too-greedy staging) — no data loss.

## Sequence

1. Unit 1 (fix) — single line change
2. Unit 2 (test) — validates the fix

No dependencies between units; could be implemented in either order. Single commit is appropriate given the small scope.

## Revision history

- rev 2 (2026-05-26): addressed F1 by adding empty-index guard (`git diff --cached --quiet`) after pathspec exclusion — skips commit and clears `RESCUED_DIRTY_WORKTREE` when only scaffold paths are dirty; addressed F2 by computing `RESCUED_FILES` from `git diff --cached --name-only` and using it in the PIPELINE FAILURE message instead of `DIRTY_FILES`; addressed F3 by replacing the stub test with two fully-implemented test functions (`test_auto_rescue_excludes_scaffold_files` and `test_auto_rescue_empty_index_guard`) covering both the mixed-content and scaffold-only scenarios.
