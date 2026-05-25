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

### Unit 1: Fix staging pathspec (dispatch-lib.sh)

**File:** `skills/bundled/_shared/dispatch-lib.sh`  
**Location:** Line ~425 (inside the Unit 1 mika#1282 dirty-worktree block)

Change:
```bash
git -C "$WORKTREE_DIR" add -A 2>&9
```

To:
```bash
git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' 2>&9
```

Add a comment explaining the exclusion:
```bash
# Stage all dirty files EXCEPT worktree-scaffold paths copied by
# _set_up_worktree (mika#1288). .claude/commands/ contains slash-command
# snapshots from mika-platform — not pilot-authored content.
git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' 2>&9
```

### Unit 2: Add test case (test-dispatch-lib.sh)

**File:** `skills/bundled/_shared/test-dispatch-lib.sh`

Add a test function that verifies the staging exclusion:

```bash
test_auto_rescue_excludes_scaffold_files() {
    # Setup: create a temp git repo simulating a worktree with:
    # 1. A scaffold file at .claude/commands/mika-groom-ticket.md (untracked)
    # 2. A pilot-authored new file at src/new_feature.rs (untracked)
    # 3. A pilot-modified tracked file (modified)
    #
    # Exercise: run the git add with pathspec exclusion
    #
    # Assert:
    # - .claude/commands/mika-groom-ticket.md is NOT staged
    # - src/new_feature.rs IS staged
    # - The modified tracked file IS staged
}
```

The test creates a temporary git repo, simulates the scenario, runs the pathspec-filtered `git add`, and asserts on `git diff --cached --name-only`.

## Acceptance criteria mapping

| Criterion | How addressed |
|-----------|--------------|
| Auto-rescue does NOT include `.claude/commands/*.md` | Pathspec `:!.claude/commands/` excludes the directory |
| Auto-rescue DOES include pilot-authored new files | `git add -A` still stages all other untracked files |
| Test verifies exclusion | Unit 2 test case |
| Re-exercise mika#897-style ticket | Manual verification (operator re-runs on a test ticket) |

## Risk assessment

**Low risk.** Single-line change to a pathspec in a rescue path that only fires on zero-commit dev-pilot exits with dirty worktrees. The pathspec syntax is well-documented git behavior. Worst case if the exclusion doesn't work: same behavior as before (too-greedy staging) — no data loss.

## Sequence

1. Unit 1 (fix) — single line change
2. Unit 2 (test) — validates the fix

No dependencies between units; could be implemented in either order. Single commit is appropriate given the small scope.
