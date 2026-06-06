---
module: skills/bundled/_shared/dispatch-lib.sh
tags: [autonomous-loop, dispatch-lib, worktree, rebase, resume, recovery, git-stash, milestone-30]
problem_type: bug
category: workflow-issues
date: 2026-06-06
ticket: mika#1414
resolution_type: structural_guard
---

# dispatch-lib: dirty-worktree-on-resume crashes the rebase — 2026-06-06

## Problem

On a **resume** dispatch (`ready`-label re-dispatch into a previously-blocked task),
`_set_up_worktree()` reuses an existing worktree and rebases it onto `origin/main`
when `BEHIND > 0`. `git rebase` refuses to run on a dirty tree:

```
error: cannot rebase: You have unstaged changes.
→ STATUS=REBASE_CONFLICT, Rebase failure mode: other, Conflicted files: <none>
```

The task re-blocks with **no recovery path** — the chain stalls indefinitely.

Confirmed n=2 on 2026-06-05 (mika#1255 `md5sum` policy-deny crash leftovers;
mika#1381 `find -exec` crash leftovers). A worktree survey that night found **13 of
17** in-flight pilot worktrees dirty — a latent stall waiting in every one. This is
milestone-30 (Loop Trustworthiness): *a loop that dispatches tickets which never ship
makes "backlog → 0" lie.*

### Root-cause nuance

The dominant re-dirtying mechanism is **`make deploy` writing a stale pre-#1255
`.claude/commands/mika.md` into worktree working trees** — a *tracked* file, so it
shows as ` M .claude/commands/mika.md`, not untracked junk. The pre-existing surgical
cleanup (mika#1301: `.iterate/`, `groom-verdict-trail.log`; mika#1311: `docs/plans/`)
did **not** cover `.claude/commands/`, so the dominant case slipped straight through
to the crashing rebase. (The companion root-cause fix — stop deploy from re-dirtying
worktrees — is the paired ticket; this fixes the *symptom* so resume survives
regardless of what dirtied the tree.)

## Fix

Extracted pre-rebase cleanup into a sourceable helper `_clean_worktree_for_rebase()`
called from the `BEHIND > 0` block. Three tiers, in order:

1. **Abort any half-finished rebase** (`git rebase --abort`) — a dispatch killed
   mid-rebase would otherwise make the stash fail and re-trigger the exact crash.
2. **Surgical resets** of dispatch-lib-owned scaffold to HEAD — the mika#1301/#1311
   paths **plus `.claude/commands/`** (the dominant deploy case). All re-copied /
   re-derived post-rebase, so resetting them costs nothing and keeps them out of the
   operator-recovery stash.
3. **Blanket fallback**: if residue survives, `git stash push --include-untracked`
   (operator-recoverable safety net), then `git reset --hard` + `git clean -fd` so the
   rebase precondition holds. `clean -fd` **omits `-x`** so gitignored config
   (`.claude/*.local.json`) survives for the post-rebase copy.

Extracting (not inlining) lets the test call the **real** function instead of an
inline copy — eliminating the Test-12e drift class where a copied guard silently
diverges from production.

## Two non-obvious bugs surfaced in review (the durable learnings)

### 1. `git rev-parse 'stash@{0}' || true` poisons the variable with a literal string

`git rev-parse` on a **missing** ref exits non-zero **but still echoes the literal
argument** (`stash@{0}`) to stdout. With `RESUME_CLEANUP_STASH=$(... || true)`, the
`|| true` swallows the failure and the variable captures the bogus string
`"stash@{0}"` — a worthless recovery handle — whenever `git stash push` reports
success yet created no entry (e.g. nothing actually stashable, a nested untracked git
repo). 

**Rule:** to capture an optional git ref, use `rev-parse --verify --quiet <ref>` —
it prints nothing and exits non-zero on a missing ref, so `$(... || true)` yields an
empty string, not a literal. Never rely on `2>/dev/null` alone; the poisoning value
goes to **stdout**, not stderr.

### 2. "Logged to stderr" ≠ "lands in the per-task stderr log file"

The groomed plan claimed the recovery line echoed to `>&2` would land in
`/var/log/claude-pilot/<id>.stderr`. **False.** That file is written *only* from the
`claude-pilot` subprocess's captured stderr (`_scrub_secrets_from_output <
"$STDERR_FILE" > "$PERSISTENT_STDERR"`, and the redirect **truncates**).
`_clean_worktree_for_rebase` runs during `_set_up_worktree()`, **before** claude-pilot
launches — its `>&2` goes to dispatch-lib's own fd 2 (the tool subprocess stderr
captured by the engine), a different stream that the per-task file never receives.

**Rule:** a per-task log file populated from a *specific subprocess's* captured
stderr does not capture the *parent script's* stderr emitted at other lifecycle
phases. The **durable, authoritative** recovery path here is the stash itself —
`git stash list` shows the entry whose message embeds the task id + UTC timestamp.
Make the recoverable artifact self-describing; treat a log echo as best-effort.

## Residual risks (accepted, documented — not fixed here)

- **Nested untracked git repo** in the worktree survives tier-3 (`stash` skips it,
  `clean -fd` without `-ff` refuses to delete nested repos) → tree stays dirty and the
  rebase can still fail honestly (with captured stderr, not a silent corruption).
  Adding `-ff` would risk deleting a legitimate nested repo (data loss), so it is
  deliberately not done. mika has no worktree submodules today.
- **Unmerged paths from a killed *merge/cherry-pick*** (not rebase) make `stash push`
  fail; the belt-and-suspenders `reset --hard` then discards uncommitted edits in the
  conflicted files with no stash handle. dispatch-lib worktrees only ever rebase, so
  this shape does not arise on the live path; committed state always survives.
- **Concurrent shared-stash race**: the stash stack is shared across a repo's
  worktrees, so a sibling dispatch pushing between this dispatch's `stash push` and
  `rev-parse` could shift `stash@{0}`. mika-dev serializes claude-pilot sessions, so
  the live path is single-flight; the immutable-SHA capture protects recovery *after*
  a correct capture.

## Verification

- `bash skills/bundled/_shared/test-dispatch-lib.sh` — 165 pass (was 162); 3 new tests:
  tier-3 stash path (+ gitignore preservation), tier-2 scaffold-only **no-stash**
  boundary (the dominant production case), tier-1 half-finished-rebase abort. The 7
  pre-existing failures are unrelated (prompt-content / case-switch / callout tests
  failing on the branch base, confirmed via stashed-baseline run).
- Shell-only, copy-deployed (not in-binary) — no `cargo build`. Rollback = revert +
  `make deploy` / re-sync bundled skill.

## Related

- mika#1255, mika#1381 — the two confirmed victims
- mika#1301, mika#1311 — prior surgical-reset cleanups this helper subsumes
- mika#1282 — post-flight dirty-worktree *content* rescue (complementary: post-pilot,
  not pre-pilot resume)
- mika#1097 — the per-task `${LOG_ID}.stderr` persistence whose scope the AC4
  misconception misjudged
