---
title: Mid-session duplicate-commit pre-push guard
date: 2026-05-26
category: logic-errors
module: dispatch-lib
problem_type: logic_error
component: tooling
symptoms:
  - PR in mergeable=CONFLICTING state despite identical content
  - git log shows commits with same diff/message but different hashes
  - Mid-session git pull creates duplicate copies of upstream commits
root_cause: missing_guard
resolution_type: code_fix
severity: medium
tags:
  - git
  - claude-pilot
  - duplicate-commit
  - cherry-mark
  - dispatch-lib
  - rebase
---

# Mid-session duplicate-commit pre-push guard

## Problem

Mid-session `git pull` or `git merge main` in a claude-pilot session can create commits that are patch-equivalent to commits already on `origin/main` but with different hashes. GitHub's 3-way merge algorithm sees both copies touching the same lines and marks the PR as `mergeable=CONFLICTING`, even though the content is identical.

The #747 rebase-or-abort guard runs once at session startup, but does not protect against duplicates introduced mid-session.

## Observed failure

PR #782 (mika#286): After the startup guard ran cleanly, the claude-pilot session ran `git pull origin main` mid-session. This created commit `8693b3fd` — a duplicate of main's `14279524` (same author date, message, diff, different hash). GitHub's 3-way merge saw both commits touching the same lines and set `mergeable=CONFLICTING`.

## Root cause

Two gaps in #747:
1. Rebase-or-abort runs once at session START — no end-of-session sanity check
2. No structural guard on what git operations the session runs mid-flight

## Solution

A `_check_duplicate_commits()` function in `dispatch-lib.sh` runs as a pre-push guard inside `_push_branch()`, before any `git push` to origin.

**Detection:** Uses `git log --cherry-mark --right-only --format="%m %H %s" origin/main...HEAD` to find commits on the branch that are patch-equivalent to commits on `origin/main`. The `--cherry-mark` flag marks equivalent commits with `=` prefix; `--right-only` limits output to the branch side.

**Self-heal:** If duplicates are detected, attempts automatic `git rebase origin/main`. Rebase naturally drops patch-equivalent commits when replaying onto a base that already has them.

**Failure mode:** If rebase fails (real conflicts, not just duplicates), returns non-zero. `_push_branch()` skips the push and appends a structured error to `RESULT` so the dispatch callback surfaces the failure.

**Failure-open on fetch:** If `git fetch origin main` fails (network, auth), the guard logs a WARN and returns 0 (skip, don't block). This surfaces degraded state in dispatch logs without blocking pushes on connectivity issues.

## Why this works

- Covers the full failure class regardless of how the duplicate arrived (pull, merge, cherry-pick, or future unknown paths)
- Runs at push time — the last structural gate before commits reach GitHub
- Auto-rebase is the same self-heal pattern as the #747 startup guard
- Failure-open design doesn't introduce new blocking failure modes

## Prevention

- When adding guards that run at session boundaries (startup, shutdown), consider whether mid-session operations can bypass them. Push-time guards are the canonical "last chance" check.
- `git log --cherry-mark` is the right tool for detecting patch-equivalent commits across branches — it compares patches (diffs), not commit hashes.

## Related issues

- [#784](https://github.com/senara-solutions/mika/issues/784) — this fix
- [#747](https://github.com/senara-solutions/mika/issues/747) — startup rebase-or-abort guard
- [#782](https://github.com/senara-solutions/mika/pull/782) — motivating PR (CONFLICTING from mid-session pull)
- `stale-base-conflicting-prs-no-self-heal-2026-04-23.md` — predecessor solution doc
