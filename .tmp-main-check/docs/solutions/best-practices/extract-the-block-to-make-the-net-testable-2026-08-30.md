---
title: "Extract an inline safety net into a function — a suite that reimplements it cannot falsify it"
date: 2026-08-30
category: best-practices
module: dispatch-lib, skills/bundled/_shared
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Widening the scope of an inline recovery/rescue block in dispatch-lib
  - Writing a test for logic that lives inside a long shell function
  - Reviewing a test suite whose assertions are `grep` over the source file
tags: [testing, shell, dispatch-lib, rescue, anti-vacuity]
---

## Problem

dispatch-lib's dirty-worktree rescue (mika#1282) lived inline inside
`_post_flight_recovery`, ~200 lines deep in an `if` chain. It could not be
called, so the four suites that covered it did one of two things:

1. **Reimplemented** the rescue in the test (`test_auto_rescue_excludes_scaffold_files`
   builds its own temp repo and runs its own `git add -A -- ':!...'`), or
2. **grepped** the source for a literal (`sed -n '/Unit 1 (mika#1282)/,/^fi$/p'`
   then `assert_contains ':!.claude/commands/'`).

Neither can falsify the shipped code. (1) tests a copy of the logic — it passes
whether or not dispatch-lib still behaves that way. (2) tests that a string is
present — it passes whether or not the string is reachable. mika#2031 is the
proof: the rescue was gated on `$SKILL = dev-pilot`, so it never ran for
`dev-groom`, and every one of those assertions was green the whole time.

## Solution

Extract the block into a named function with its guards at the top, then call
the real function from the test against a real temp git repo:

```bash
_rescue_dirty_worktree() {
    [ -n "$WORKTREE_DIR" ] && [ -n "$REPO" ] || return 0
    case "$SKILL" in dev-pilot|dev-groom) ;; *) return 0 ;; esac
    [ "${PRE_RUN_HEAD:-}" = "${POST_RUN_HEAD:-}" ] || return 0
    ...
}
```

```bash
source "$DISPATCH_LIB"
exec 9>/dev/null          # dispatch-lib redirects git noise to fd 9
WORKTREE_DIR="$repo"; SKILL="dev-groom"; ...
_rescue_dirty_worktree || true
assert_eq "plan is tracked at HEAD" 1 "$(git -C "$repo" ls-tree -r --name-only HEAD | grep -c plan)"
```

The extraction is a pure move — the same body, one indent level shallower — so
it is reviewable as a move, and the behavior change (one guard) is the small
readable part of the diff.

## Two things that make the new suite worth having

**Assert the no-op, not only the action.** A rescue that fires unconditionally
passes every dirty-tree assertion. The suite asserts a *clean* tree produces no
commit, no note, and no marker — for each skill — so "always fires" and "fires
when it should" stop being indistinguishable from inside the suite.

**Run the negative control before believing the suite.** Flipping the guard back
to `dev-pilot` only and re-running turned 29/29 into 21/29. A suite that stays
green with the fix reverted is measuring something else.

## Structural anchors are a real coupling — re-anchor, never relax

Four existing assertions grep dispatch-lib for the literal
`Unit 1 (mika#1282): detect dirty worktree`, and one counts
`git -C "$WORKTREE_DIR" commit -m "wip(` occurrences (expects exactly 3). A
refactor that renames a comment or interpolates a variable *into* that literal
silently breaks them — and `set -euo pipefail` in test-dispatch-lib.sh makes the
suite die mid-run with no `✗` printed, so the failure reads as "the suite
stopped" rather than "an assertion failed".

The right move both times is to keep the assertion and fix its address:

- the call-site comment keeps the exact `Unit 1 (mika#1282): detect dirty
  worktree` phrasing, so the four line-order guards still resolve;
- the commit subject stays `commit -m "wip(${REPO}#${ISSUE_NUM}): ${_var}` —
  interpolation *after* the literal, not around it;
- the one anchor that genuinely moved (block extraction by `sed` range) is
  re-pointed at `/^_rescue_dirty_worktree() {/,/^}$/`, same assertions.

Relaxing an assertion to make a refactor pass converts a guard into decoration.

## Wire the new suite into the target CI actually runs

`make test` lists suites one by one, and CI runs `make test-dispatch-lib` — which
ran exactly one file. A new suite under `skills/bundled/_shared/tests/` is not
picked up by a glob; it has to be added to both. A guard that never runs is not
a guard.

## Related

- mika#2031 — dev-groom dirty-worktree rescue (this work)
- mika#1282 — the original rescue; mika#1296 / mika#1341 / mika#1685 — its
  failure-path hardening
- `docs/solutions/best-practices/verify-a-bulk-rewrite-with-a-guard-that-stays-2026-08-29.md`
  (mika-platform) — same shape, different layer
