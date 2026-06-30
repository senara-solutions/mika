---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
origin: mika#1674
deepened: 2026-06-30
---

# chore(.gitignore): untrack `.claude/groom-verdict-trail.log`

## Summary

`.claude/groom-verdict-trail.log` is an operator-local audit log of grooming
verdicts, accidentally tracked in git. Untrack it (`git rm --cached`, preserving
the local file) and add it to `.gitignore` so future pilot/operator sessions
can't commit it. Two-line change; no code, no behavior change.

---

## Problem Frame

The file accumulates rows of `<timestamp>\t<gate>\t<session-id>\t<disposition>`
as the operator runs `/mika-groom-ticket` — local diagnostic state, not shared
substrate. It is tracked in git: `git ls-files` lists it and 40+ commits since
May 2026 have touched it (latest `cb03dadd`, PR #1669). Every unrelated PR picks
up incidental 2-line diffs to this file, obscuring real changes.

**Why now:** cleanliness of PR diffs. Loop-impact is minimal (the file is not
load-bearing), so this is a tier-5 substrate-hygiene chore.

---

## Requirements

- R1. The file is removed from git tracking without deleting the operator's local copy.
- R2. `.gitignore` prevents the file from being re-added in future commits.
- R3. The working tree is clean after the change (no rogue tracking).

Out of scope (carried from mika#1674):
- History rewriting to scrub the file from past commits.
- Generalizing to other `.claude/*.log` patterns — file separately if a class emerges.

---

## Key Technical Decisions

- **Exact-path ignore, not a glob.** Add the literal `.claude/groom-verdict-trail.log`
  rather than `.claude/*.log`. The ticket scopes to this one file; a broad glob
  risks silently ignoring future `.claude/*.log` files that *should* be tracked,
  and the out-of-scope note explicitly defers the pattern-class decision. Mirrors
  the existing exact-path style of the `.claude/` block (`scheduled_tasks.lock`,
  `worktrees`).
- **`git rm --cached`, not `git rm`.** The `--cached` flag detaches the file from
  the index while leaving it on disk, satisfying R1 and R3 without destroying the
  operator's local audit data (R-preservation, AC3).

---

## Implementation Units

### U1. Untrack the file and ignore it

**Goal:** Stop tracking `.claude/groom-verdict-trail.log` and prevent re-addition.

**Requirements:** R1, R2, R3

**Dependencies:** none

**Files:**
- `.gitignore` — add `.claude/groom-verdict-trail.log` inside the existing `.claude/` block (after `.claude/scheduled_tasks.lock`, line ~34).
- `.claude/groom-verdict-trail.log` — `git rm --cached` (index removal only; file stays on disk).

**Approach:**
1. Append the ignore line to the `.claude/` group in `.gitignore`.
2. `git rm --cached .claude/groom-verdict-trail.log`.
3. Stage both changes. The resulting commit shows the file deleted from the index
   plus the one-line `.gitignore` addition.

**Patterns to follow:** existing `.claude/` ignore entries in `.gitignore`
(`.claude/*.local.json`, `.claude/worktrees`, `.claude/scheduled_tasks.lock`).

**Test scenarios:** Test expectation: none — pure repo-hygiene change with no
code path or behavior. Verification is by git state inspection (see Verification
Contract), not by a test suite.

**Verification:** `git ls-files .claude/groom-verdict-trail.log` returns empty;
the local file still exists on disk; `git status` shows only the staged
deletion + `.gitignore` edit and is otherwise clean.

---

## Verification Contract

- `git ls-files .claude/groom-verdict-trail.log` → empty output (AC1).
- `grep -F '.claude/groom-verdict-trail.log' .gitignore` → matches (AC2).
- `test -f .claude/groom-verdict-trail.log` → exists (AC3).
- After staging, `git status --porcelain` shows only the `.gitignore` modification
  and the index removal of the log file; re-touching the local log produces no new
  untracked/modified entry for it (AC4).

---

## Definition of Done

- The file is untracked, the local copy preserved, the ignore rule in place, and
  the working tree clean. PR opened against `senara-solutions/mika` closing #1674.

## Acceptance criteria

- [ ] AC1. `.claude/groom-verdict-trail.log` is removed from `git ls-files` output.
- [ ] AC2. `.gitignore` contains `.claude/groom-verdict-trail.log` (or pattern equivalent like `.claude/*.log`).
- [ ] AC3. Local file at `.claude/groom-verdict-trail.log` continues to exist (the `--cached` flag preserves it).
- [ ] AC4. After commit, `git status` is clean (no rogue tracking of the file).
