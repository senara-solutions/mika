---
title: "fix(dispatch): auto-rescue commit silently fails in linked worktrees — .git-as-file scratch path"
type: fix
status: active
date: 2026-05-30
issue: senara-solutions/mika#1341
branch: bug/1341/dispatch-auto-rescue-commit-silently
---

# fix(dispatch): auto-rescue commit silently fails in linked worktrees (.git-as-file scratch path / ENOTDIR)

## Summary

The autonomous loop's dirty-worktree auto-rescue (`skills/bundled/_shared/dispatch-lib.sh`, mika#1282/#1296/#1310) writes its commit-output capture file to `"$WORKTREE_DIR/.git/mika-rescue-commit-err"`. In a **linked git worktree** — which is every autonomous `dev-pilot` run (`.claude/worktrees/<branch>/<repo>`) — `.git` is a **file** (a `gitdir:` pointer), not a directory. The redirect `git commit ... > "$RESCUE_COMMIT_ERR" 2>&1` therefore cannot open its target (`ENOTDIR`), the redirection fails, and per POSIX shell semantics **the `git commit` is never executed and exits non-zero**. The rescue silently produces no commit, no branch push, and no PR.

Fix: one-line change to write the scratch file to a guaranteed-valid path (`mktemp`). Plus a regression test that exercises the rescue path inside a real linked worktree.

This is a **Lightweight** plan: one production-line fix + one regression test. No scope expansion.

---

## Problem Frame

**Observed (cpp#24 → mika#1341):** A fully-groomed ticket re-dispatched through the loop on 2026-05-29 produced `PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt)` with `Hook output: <rescue capture was empty>`. The pilot's edit (`M Dockerfile.agent`) was present but uncommitted; no PR.

**Root cause (confirmed by live repro):**
- `dispatch-lib.sh:512` — `RESCUE_COMMIT_ERR="$WORKTREE_DIR/.git/mika-rescue-commit-err"`.
- In a linked worktree, `.git` is a 79-byte file (`gitdir: /…/.git/worktrees/<name>`), not a directory. Live repro: `echo probe > <linked-wt>/.git/mika-rescue-commit-err` → `not a directory` (exit 1).
- The `> "$RESCUE_COMMIT_ERR" 2>&1` redirect on the `git commit` (`dispatch-lib.sh:522`) fails to open → `git commit` never runs → exits non-zero with no output written.

**Symptom chain (each maps to one observed fact):**
| Observed | Mechanism |
|---|---|
| HEAD unchanged, edit left uncommitted, no PR | `git commit` never executes (redirect open fails) |
| `rejected by pre-commit hook (non-rustfmt)` | `if git commit` → exit≠0 → elif `grep rustfmt "$RESCUE_COMMIT_ERR"` → file absent → no match → non-rustfmt `else` branch (`dispatch-lib.sh:607`) |
| `<rescue capture was empty>` | `cat "$RESCUE_COMMIT_ERR"` → file never created → empty → git-status diagnostic fallback (mika#1310) |

**Ruled out (with evidence — do not chase):**
- *Injected `.claude/commands` pollution*: already excluded by mika#1288 via `git add -A -- ':!.claude/commands/'` (`dispatch-lib.sh:466`/`:516`). Never enters the changeset; appears only in the diagnostic dump. Red herring.
- *lefthook missing from PATH*: `.git/hooks/pre-commit` falls through to `echo "Can't find lefthook in PATH"` and returns 0 (non-fatal). With only `Dockerfile.agent` staged, no `lefthook.yml` job glob even matches. Never the rejecter.

**Provenance:** introduced by mika#1296 (commit `0eababa7`, 2026-05-26). Latent 3 days because mika#1327's groom brake wedged the loop upstream until 2026-05-29 (this is the "next downstream wedge").

---

## Scope Boundaries

**In scope:**
- Change the rescue commit-capture scratch path to a guaranteed-valid location.
- Add a regression test covering the rescue path under a linked worktree.

**Out of scope (non-goals):**
- The deeper Layer-1 question of *why the pilot exits 0 without self-committing* (the rescue is a safety net; this fix makes the net work). Tracked separately if it recurs.
- Refactoring the rescue block's structure, the mika#1310 diagnostic logic, or the lefthook/pre-commit hook.

### Deferred to Follow-Up Work
- The existing `test_rescue_hook_failure_invariant` (test-dispatch-lib.sh ~line 880) **reimplements** the rescue logic inline rather than sourcing the real function, and uses `2>"$RESCUE_COMMIT_ERR"` (stderr-only) instead of the real `> … 2>&1`. Tightening these tests to exercise the real `dispatch_claude_pilot` function is a larger test-architecture change — not pulled into this fix.

---

## Key Technical Decisions

**KTD-1: Use `mktemp` for the scratch path.**
`RESCUE_COMMIT_ERR="$(mktemp)"` — out of the working tree (preserves mika#1296's intent of not coupling to `.iterate/`), always a real path regardless of linked vs. non-linked worktree, no git-internals writing. The existing `rm -f "$RESCUE_COMMIT_ERR"` cleanup in every branch works unchanged (a `mktemp` file is a normal file).

*Alternative considered:* `"$(git -C "$WORKTREE_DIR" rev-parse --git-dir)/mika-rescue-commit-err"` resolves to the real per-worktree git dir (`<repo>/.git/worktrees/<name>`, a real directory). Rejected: more complex, writes into git internals, and adds a subprocess; `mktemp` is simpler and strictly safer. No reason found to prefer it.

**KTD-2: Single assignment site.** There is exactly one `RESCUE_COMMIT_ERR=` assignment (`dispatch-lib.sh:512`) feeding all three commit-attempt paths (first commit, cargo-fmt retry, and both failure branches). The one-line change fixes every path; the comment above it (lines 510–512 referencing `.git/`) must be updated to reflect the new rationale.

---

## Implementation Units

### U1. Fix the rescue scratch path

**Goal:** Replace the linked-worktree-invalid `.git/` scratch path with a `mktemp` path so the rescue `git commit` redirect always opens.

**Requirements:** Acceptance criterion 1 (rescue produces a real commit + push + PR in a linked worktree).

**Dependencies:** none.

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (modify — the `RESCUE_COMMIT_ERR=` assignment at line 512 and its preceding comment at lines 510–512)

**Approach:**
- Change `RESCUE_COMMIT_ERR="$WORKTREE_DIR/.git/mika-rescue-commit-err"` → `RESCUE_COMMIT_ERR="$(mktemp)"`.
- Update the comment to explain the linked-worktree `.git`-is-a-file hazard and cite mika#1341 (so a future reader doesn't "restore" the `.git/` location for orthogonality reasons).
- Confirm all `rm -f "$RESCUE_COMMIT_ERR"` cleanup sites remain correct (three branches: first-try success, retry success, both failure branches). No new exit path is introduced.

**Patterns to follow:** mika#1296/#1310 comment-citation style already in this block (inline `# mika#NNNN:` rationale comments).

**Test scenarios:** behavior is covered by U2's regression test. No unit-local test beyond U2.

**Verification:** `grep -n 'RESCUE_COMMIT_ERR=' skills/bundled/_shared/dispatch-lib.sh` shows the `mktemp` form and no `.git/` path; `bash -n` parses clean.

### U2. Regression test: rescue path under a linked worktree

**Goal:** Lock in the fix with a test that fails against the `.git/`-scratch code and passes after U1, and that documents the linked-worktree root cause as an executable artifact.

**Requirements:** Acceptance criterion 2.

**Dependencies:** U1 (the structural assertion asserts the fixed shape).

**Files:**
- `skills/bundled/_shared/test-dispatch-lib.sh` (modify — add a test block mirroring the existing Test A/Test B structure near the mika#1296 rescue tests, ~line 845+)

**Approach (mirror existing A/B convention):**
- **Test A — structural (coupled to real source):** extract the `RESCUE_COMMIT_ERR=` assignment line from `dispatch-lib.sh` and assert it does **not** contain `.git/mika-rescue-commit-err` and **does** use `mktemp`. This is the assertion that fails on current code and passes after U1.
- **Test B — live invariant (linked-worktree proof):** in a `mktemp -d` base repo with an initial commit, create a **linked worktree** via `git worktree add`. Assert that `.git` inside the linked worktree is a file (`[ -f "$wt/.git" ]`), that a redirect into `"$wt/.git/scratch"` fails (documents ENOTDIR), and that a `git commit` capturing into a `mktemp` path **succeeds and advances HEAD** with a dirty tracked file staged. Clean up the worktree (`git worktree remove --force`) and temp dirs via `trap … RETURN`.

**Patterns to follow:** existing `test_auto_rescue_empty_index_guard` and `test_rescue_hook_failure_invariant` (function returning `PASS`/`FAIL: …`, wrapped by a `RESULT_X=$(…)` + `assert`-style PASS/FAIL counter increment). Use `git -C` throughout; no `cd` into the harness's own tree.

**Test scenarios:**
- Covers AC2. Structural: `RESCUE_COMMIT_ERR` assignment uses `mktemp`, not `$WORKTREE_DIR/.git/`. (Fails pre-fix.)
- Linked worktree: `.git` is a file; redirect into `<wt>/.git/<name>` fails (exit ≠ 0).
- Linked worktree: rescue commit with a `mktemp` capture path succeeds; `HEAD` advances; staged file is committed; scratch file is cleaned up.

**Verification:** `bash skills/bundled/_shared/test-dispatch-lib.sh` — all assertions pass on the fixed code; the structural assertion fails when temporarily reverting U1.

---

## Risks & Dependencies

- **Low risk.** One-line production change in a well-isolated rescue path; `mktemp` is POSIX-portable and already used throughout the test harness. No schema, API, or interface contract touched.
- **Deploy note:** `dispatch-lib.sh` is copy-deployed (not in-binary) — the fix is live only after `make deploy` re-seeds bundled skills to `~/.mika`. Per `project_dispatch_lib_deploy_lag_wedge`: diff the `~/.mika` copy against source after merge to confirm the live copy carries the fix before declaring the loop unwedged.

## Acceptance Criteria

1. A pilot that writes files but does not self-commit, running in a linked worktree, is auto-rescued into a real commit + pushed branch + PR — no "non-rustfmt empty-capture" PIPELINE FAILURE.
2. `test-dispatch-lib.sh` covers the rescue path under a linked worktree and fails against the current `.git/`-scratch code.

## Sources & Research

- Live repro (this session): linked-worktree `.git` is a file; `echo probe > <wt>/.git/<name>` → `not a directory`.
- `git blame` / `git log -S`: scratch path introduced by mika#1296 (`0eababa7`, 2026-05-26).
- `dispatch-lib.sh:466`/`:516` (mika#1288 pathspec exclusion); `.git/hooks/pre-commit` + `lefthook.yml` (lefthook non-fatal fallthrough).
- Originating mis-scoped report: claude-pilot-py#24 (closed, pointing here).
