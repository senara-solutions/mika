---
title: Post-flight rescue commits must bypass the pre-commit hook — one clippy nit wedged the salvage path
tags:
  - dispatch-lib
  - autonomous-loop
  - lefthook
  - pre-commit
  - clippy
  - rescue
  - dev-pilot
module: skills/bundled/_shared/dispatch-lib.sh
problem_type: workflow_issue
category: workflow-issues
severity: high
created: 2026-06-30
---

# Post-flight rescue commits must bypass the pre-commit hook — one clippy nit wedged the salvage path

## Problem

dispatch-lib's post-flight rescue paths (mika#1282 dirty-worktree, mika#1383 trailing-content) auto-commit a claude-pilot's uncommitted work into a draft PR so a 29-turn, ~$4 pilot session isn't lost when it ends with a dirty worktree. Those `git commit` calls ran the repo's pre-commit hook (lefthook → `rust-clippy -D warnings`). A single clippy nit left in the worktree by the pilot rejected the rescue commit, so the salvage never landed and the parent task went `blocked` — the 95%-complete work stranded with no PR. Modal cause of a 6-task loop wedge on 2026-06-30 (n=3+ confirmed: tasks `dd81ff3b`/mika#1680, `48d03390`/mika#1682, `12f621bc`/mika#1676).

## Symptoms

- Parent dispatch task transitions `in_progress → blocked`.
- Identical callback result prefix across affected tasks:
  `PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook after cargo-fmt retry.`
- Worktree left dirty for operator inspection; no draft PR opens.
- The leftover clippy error is typically trivial (e.g. `std::iter::repeat(' ').take(300).collect()` should be `" ".repeat(300)`).

## What Didn't Work (the trap that hid the root cause)

dispatch-lib already had a reactive rescue retry: on a rejected commit it checked `grep "rust-fmt\|cargo fmt\|rustfmt"` against the captured hook output and, on a match, ran `cargo fmt --all` and re-committed. That retry could **never** fix a clippy rejection — `cargo fmt` reformats, it does not satisfy `clippy -D warnings`. The retry fired anyway and hit the same wall, producing the "after cargo-fmt retry" failure message.

## Why the clippy failure hit the fmt-retry branch (the non-obvious mechanism)

`lefthook.yml` runs `rust-fmt` **and** `rust-clippy` as sibling pre-commit commands. lefthook prints **all** step names in its decoration block (the `2>&1`-captured output dispatch-lib greps), including `rust-fmt`, even when **clippy** is the step that actually failed. So a pure clippy rejection still matched `grep "rust-fmt"`, routed down the fmt-retry path, and surfaced the misleading "cargo-fmt retry" message. The grep was keying on a step *name present in the output*, not the step that *failed* — a classic "the log mentions X, therefore X is the cause" misread baked into control flow.

## Solution

Add `--no-verify` to all three rescue `git commit` invocations in `skills/bundled/_shared/dispatch-lib.sh` (mika#1685). The rescue path's purpose is to **salvage work for operator review**, not to gate it on lint — a one-line typo must not strand a $4 pilot. Lint still surfaces at the right layer: CI re-runs `cargo fmt --check` + `clippy` on the resulting draft PR (`ci.yml`), and `wip-staleness-check.yml` re-clippies `wip-rescue` drafts when main moves.

```sh
# before
git -C "$WORKTREE_DIR" commit -m "wip(...): impl staged by post-flight recovery (mika#1282) ..." > "$RESCUE_COMMIT_ERR" 2>&1
# after
git -C "$WORKTREE_DIR" commit -m "wip(...): impl staged by post-flight recovery (mika#1282) ..." --no-verify > "$RESCUE_COMMIT_ERR" 2>&1
```

`--no-verify` placed after the multi-line `-m` value is parsed correctly: `-m` consumes exactly the one quoted message argument, then `--no-verify` is an option to `git commit`. The pre-existing `rust-fmt` reactive-retry branch becomes effectively unreachable on hook grounds once the initial commit bypasses the hook; it was kept-and-noted (mika#1685 AC4) defensively rather than removed.

## Trade-off (accepted, tracked)

`git commit --no-verify` is **all-or-nothing** — it skips the *entire* lefthook pre-commit block, not just lint. That block also runs `no-secrets` (API-key / private-key regex scan) and `no-large-files` (1 MB cap), and **CI does not replicate those**. So the rescue path no longer secret-scans before pushing the branch to origin. Accepted because: the rescue output is a **draft** PR (operator-gated, never auto-merged), the secret-prone scaffold paths (`.claude/*.local.*`, `claude-pilot.json`, `.claude/commands/`) are already excluded from rescue staging (mika#1288/#1419), and secrets are scrubbed at the engine DB/tool-call layer. A CI secret-scan net is tracked as a follow-up: **mika#1689**.

A surgical alternative (skip only the lint commands via `LEFTHOOK_EXCLUDE=rust-fmt,rust-clippy,...`) was rejected: it keys on lefthook-specific env semantics and hard-codes the lint command names, so it would **re-wedge** the moment a new lint command is added to `lefthook.yml`. `--no-verify` is future-proof against pre-commit config drift, which matters more for a loop-substrate unblock than preserving the secret scan on this one fallback path.

## Prevention

- **A rescue/salvage path's job is to preserve work for review, not to enforce quality gates.** Quality belongs on the PR (CI), where it's a check rather than a hard block that can strand work. When you find a lint gate on a salvage path, that's usually the bug.
- **Don't grep a multi-step hook's combined output for a step *name* to infer the *failing* step.** lefthook (and most parallel hook runners) print every step's name; a name in the output proves the step ran, not that it failed. If you must branch on which step failed, key on per-step exit status, not on string presence in the decoration block.
- **`--no-verify` is all-or-nothing** — before reaching for it, enumerate every command the pre-commit hook runs (`lefthook.yml` here) and confirm nothing load-bearing (secret scan, large-file guard) is silently dropped. If something is, either accept-and-track it or move that check to a layer the bypass can't skip.
- Regression guard lives at `skills/bundled/_shared/tests/test_rescue_commit_no_verify.sh`: proves a rejecting pre-commit hook blocks a bare commit but not `--no-verify`, and statically asserts all three rescue sites still carry the flag.

## References

- mika#1685 — this fix
- mika#1689 — follow-up: add a CI secret-scan net for `wip-rescue` draft PRs
- mika#1282 — original dirty-worktree rescue
- mika#1383 — commit-pushed-no-pr / trailing-content rescue
- mika#1296 — the cargo-fmt reactive-retry mechanism (now defensive)
- mika#1058 — callback-can't-retry-pilot trap (downstream of this cause; separate)
- `skills/bundled/_shared/lefthook.yml` — the pre-commit command set
- `.github/workflows/ci.yml`, `.github/workflows/wip-staleness-check.yml` — where clippy re-surfaces on the draft PR
