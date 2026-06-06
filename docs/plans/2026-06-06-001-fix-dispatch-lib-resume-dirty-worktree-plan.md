---
issue: mika#1414
type: fix
title: "detect dirty-worktree on resume, route to recovery contract instead of crashing rebase"
branch: fix/1414/dispatch-lib-detect-dirty-worktree-on
status: groomed
date: 2026-06-06
component: dispatch-lib (skills/bundled/_shared/dispatch-lib.sh)
milestone: milestone-30 (Loop Trustworthiness)
---

# Plan — mika#1414: dirty-worktree detection on resume before rebase

## Problem (verified in code)

`_set_up_worktree()` in `skills/bundled/_shared/dispatch-lib.sh` reuses an existing
worktree on a resume dispatch (lines 260-289), then runs the rebase-or-abort guard
(lines 291-339). The guard does a **surgical** cleanup of three dispatch-lib-owned
paths before `git rebase origin/main`:

- `git checkout -- .claude/groom-verdict-trail.log` (line 301, mika#1301)
- `rm -rf .iterate/` (line 302, mika#1301)
- `git checkout HEAD -- docs/plans/` (line 311, mika#1311 follow-up)

If the worktree carries **any other** dirty state, `git rebase origin/main` aborts
with `error: cannot rebase: You have unstaged changes.` → `STATUS=REBASE_CONFLICT`
with `Rebase failure mode: other`, `Conflicted files: <none>`. The task re-blocks
with no recovery path. Confirmed n=2 on 2026-06-05 (mika#1255, mika#1381); n=13
latent across in-flight worktrees.

### Root-cause nuance discovered during planning (informs the fix shape)

`.claude/commands/mika.md`, `.claude/claude-pilot.json`, and
`.claude/groom-verdict-trail.log` are **tracked files** in the mika repo
(verified: `git ls-files .claude/`). The dominant re-dirtying mechanism — the
sibling ticket's `make deploy` writing a stale `mika.md` into worktree working
trees — therefore surfaces as a **modified tracked file** (` M .claude/commands/mika.md`),
not as untracked junk. The existing surgical list does **not** cover
`.claude/commands/`, so this dominant case slips straight through to the crashing
rebase. The other observed cases (md5sum / `find -exec` crash leftovers) are
arbitrary residue the surgical list also cannot anticipate.

This splits the fix into two tiers (see Approach): a **surgical extension** for the
dominant dispatch-lib-owned `.claude/commands/` case (no stash noise, since
dispatch-lib re-copies it at line 358 anyway), and a **blanket fallback** for the
genuinely-unexpected residue (stash-as-safety-net, surfaced for operator recovery).

## Scope

- **In scope:** the resume/rebase path inside `_set_up_worktree()` — i.e. the
  `if [ "$BEHIND" -gt 0 ]` block (lines 293-339). The crash only occurs when a
  rebase is attempted (BEHIND > 0); a dirty worktree with BEHIND = 0 attempts no
  rebase and does not crash here.
- **Out of scope:** the root-cause sibling (`make deploy` re-dirtying worktrees
  with stale `mika.md`) — that is the paired ticket referenced in the issue's
  "Sibling fix" section; this ticket fixes the **symptom** (resume must survive a
  dirty worktree) so the chain stops stalling regardless of what dirtied it.
- **Out of scope:** the post-pilot dirty-worktree *content rescue* (mika#1282,
  lines 485-636). That fires after the pilot runs to rescue authored-but-uncommitted
  content into a draft PR. This ticket is about the *pre-pilot resume* cleanup so
  the rebase can proceed at all. The two are complementary, not overlapping.

## Approach

Extract the pre-rebase cleanup into a single sourceable helper
`_clean_worktree_for_rebase()` and call it from `_set_up_worktree()` immediately
before the rebase attempt. The helper does, in order:

1. **Abort any half-finished rebase (hardening).** If a prior dispatch was killed
   mid-rebase, the worktree is left in a rebase-in-progress state that would make
   the stash below fail and re-trigger the exact crash this ticket fixes:
   ```sh
   git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
   ```

2. **Surgical resets (migrated from current lines 301-311, extended).** Reset the
   dispatch-lib-owned ephemeral/scaffold paths to HEAD. **Add `.claude/commands/`**
   to the list — it is dispatch-lib-owned scaffold (mika#1288), re-copied fresh at
   line 358 on every dispatch, so resetting it costs nothing and removes the
   dominant deploy-re-dirty case without polluting the operator-recovery stash:
   ```sh
   git -C "$WORKTREE_DIR" checkout -- .claude/groom-verdict-trail.log 2>/dev/null || true
   rm -rf "$WORKTREE_DIR/.iterate" 2>/dev/null || true
   git -C "$WORKTREE_DIR" checkout HEAD -- docs/plans/ 2>/dev/null || true
   git -C "$WORKTREE_DIR" checkout HEAD -- .claude/commands/ 2>/dev/null || true
   ```

3. **Blanket fallback for unexpected residue.** Re-check
   `git status --porcelain`. If still non-empty, the residue is genuinely
   unexpected (crash leftovers, new untracked files deploy added). Stash it as a
   safety net, then hard-reset + clean so the rebase precondition holds:
   ```sh
   if [ -n "$(git -C "$WORKTREE_DIR" status --porcelain 2>/dev/null)" ]; then
       STASH_MSG="dispatch-lib-resume-cleanup-${LOG_ID}-$(date -u +%Y%m%dT%H%M%SZ)"
       if git -C "$WORKTREE_DIR" stash push --include-untracked -m "$STASH_MSG" >/dev/null 2>&1; then
           # Capture the IMMUTABLE stash commit SHA (not the positional stash@{0},
           # which shifts as other worktrees push/pop on the shared stash stack).
           RESUME_CLEANUP_STASH=$(git -C "$WORKTREE_DIR" rev-parse stash@{0} 2>/dev/null || true)
           echo "dispatch-lib: resume-cleanup stashed dirty worktree before rebase → stash ${RESUME_CLEANUP_STASH:-<unknown>} (msg: ${STASH_MSG}); recover with: git -C ${WORKTREE_DIR} stash apply ${RESUME_CLEANUP_STASH:-<sha>}" >&2
       else
           echo "dispatch-lib: resume-cleanup stash failed (nothing to stash or stash error); proceeding with hard reset" >&2
       fi
       # Belt-and-suspenders: ensure a clean tree even if stash captured nothing
       # (e.g. unmerged paths). Does NOT remove gitignored files (no -x), so the
       # gitignored .claude/*.local.json is preserved for the line-341 config copy.
       git -C "$WORKTREE_DIR" reset --hard HEAD 2>/dev/null || true
       git -C "$WORKTREE_DIR" clean -fd 2>/dev/null || true
   fi
   ```

4. The existing rebase attempt (lines 313-338) runs unchanged on the now-clean tree.

### Why stash is safe with the tracked/gitignored split (verified)

- `.claude/*.local.json`, `.claude/worktrees`, `.claude/scheduled_tasks.lock` are
  gitignored. `git stash --include-untracked` does **not** capture ignored files
  (would need `--all`) and `git clean -fd` does **not** remove them (would need
  `-x`). So `settings.local.json` survives and is re-copied at line 344 regardless.
- `.claude/claude-pilot.json` and `.claude/commands/` are tracked → reset to HEAD
  by step 2, then re-copied fresh from `$PLATFORM_DIR` at lines 343/358 after the
  rebase. No loss.

## AC mapping

| AC (reconciled where noted) | Where satisfied |
|----|-----------------|
| AC1 — detects dirty-worktree before rebase on resume path | Step 3 `git status --porcelain` check inside `_clean_worktree_for_rebase()`, called before the rebase attempt |
| AC2 (reconciled) — auto-stashes *unexpected residue* with descriptive name `dispatch-lib-resume-cleanup-<task_id>-<timestamp>` | Step 3 `STASH_MSG` uses `$LOG_ID` (task id) + UTC timestamp; fires only on residue after surgical reset (see Spec reconciliation) |
| AC3 — rebase proceeds on clean state | Steps 1-3 leave the tree clean; existing rebase at line 316 then succeeds |
| AC4 (reconciled) — stash ref logged to durable per-task stderr log for operator recovery | Step 3 echoes the **immutable stash commit SHA** + `git stash apply` command to stderr → `/var/log/claude-pilot/${LOG_ID}.stderr` (mika#1097). See Spec reconciliation |
| AC5 (reconciled) — reproducer: dirty `git status --porcelain` (unexpected residue) + new dispatch → no rebase crash; stash ref logged | New test in `test-dispatch-lib.sh` dirties a non-scaffold tracked file + untracked file (see Testing) |

## Spec reconciliation (AC2 / AC4 / AC5) — committed, issue body to be updated

The two-tier design and the durable-stderr logging are architecturally sound but
reframe three ACs from their literal wording. Per the project's spec-divergence
workflow (issue body is updated to match an architect-endorsed plan), these are
committed reframes; the issue body ACs will be updated to match in Phase 5 (the
edited ACs are recorded below):

- **AC2 / AC5 — "Auto-stashes" / "stash ref logged" → "stash only unexpected
  residue."** Decision: the resume stashes **only the dirt that remains after the
  surgical resets** (step 2). Routine dispatch-lib-owned scaffold/ephemeral paths
  (`.claude/commands/`, `.claude/groom-verdict-trail.log`, `.iterate/`,
  `docs/plans/`) are reset to HEAD, not stashed, because dispatch-lib re-copies /
  re-derives them post-rebase anyway and stashing them would fill operator-recovery
  stashes with noise. **Consequence (accepted):** when the only dirt is a
  surgically-handled path (e.g. a stale `mika.md` from deploy), the resume cleans +
  rebases with **no stash created** — correct, because there is nothing
  non-recoverable to preserve. The stash + log fire only when genuinely unexpected
  residue is present. Reconciled AC2/AC5 wording: *"When unexpected dirty residue
  remains after surgical reset, auto-stash it with descriptive name
  `dispatch-lib-resume-cleanup-<task_id>-<timestamp>` and log the stash ref for
  operator recovery."*

- **AC4 — "Stash ref logged in task payload" → "operator-recoverable via the
  stash itself."** Decision: on the success path the RESULT "task payload" is owned
  by the pilot outcome (lines 454-462), assembled long after worktree setup;
  threading the setup-phase stash into a success RESULT would mean restructuring
  result assembly, out of proportion to the fix.

  **Implementation correction (post-review, mika#1414):** the original draft of this
  bullet claimed the stash SHA emitted to stderr lands in
  `/var/log/claude-pilot/${LOG_ID}.stderr`. That is **false** and was corrected
  during code review. That file is written *only* from the `claude-pilot`
  subprocess's captured stderr (`_scrub_secrets_from_output < "$STDERR_FILE" >
  "$PERSISTENT_STDERR"`), and the redirect truncates (`>`); `_clean_worktree_for_rebase`
  runs during `_set_up_worktree()`, **before** claude-pilot launches, so its `>&2`
  echo goes to dispatch-lib's own fd 2 (the tool subprocess stderr captured by the
  engine), not to that per-task file. The **durable, authoritative** operator-recovery
  path is therefore the stash itself: `git -C <worktree> stash list` shows the entry,
  whose message embeds `${LOG_ID}` (task id) + UTC timestamp, and the immutable SHA is
  captured in `RESUME_CLEANUP_STASH`. The stderr echo is a best-effort convenience that
  lands wherever the engine routes dispatch-lib stderr. Reconciled AC4 wording:
  *"Stash ref recoverable via `git stash list` (message embeds task id + timestamp);
  recovery command additionally echoed to dispatch-lib stderr."*

## Testing

Add `test_resume_dirty_worktree_cleaned()` to
`skills/bundled/_shared/test-dispatch-lib.sh`:

1. `_fixture_setup` + `_assert_fixture_is_local`.
2. Create `feat/resume-dirty` branched one commit back, push it; advance `origin/main`
   one **non-conflicting** commit (so BEHIND > 0 and a clean rebase is possible).
3. Dirty the worktree: modify a tracked file (e.g. `echo x >> file.txt`) **and**
   create an untracked file (`echo y > junk.tmp`).
4. `source` dispatch-lib and call `_clean_worktree_for_rebase` against the fixture
   worktree (real code — no inline copy, eliminating the Test-12e drift risk), then
   run the rebase.
5. Assertions:
   - `git status --porcelain` is empty after cleanup (precondition held).
   - rebase exits 0 (no `STATUS=REBASE_CONFLICT`).
   - `git stash list` contains an entry whose message matches
     `dispatch-lib-resume-cleanup-*` (stash safety net created).
   - the stashed content is recoverable (`git stash show` non-empty / contains the
     dirtied paths).

Run: `bash skills/bundled/_shared/test-dispatch-lib.sh` (full suite must stay green).
`cargo build` is not required — this is a shell-only change.

## Committed decisions

1. **Extract, not inline (committed).** The pre-rebase cleanup is extracted into
   `_clean_worktree_for_rebase()` rather than inlined at the call site. Rationale:
   (a) the test calls the **real** function instead of copying the guard (Test 12e
   currently inline-copies the rebase guard and accepts drift risk — this stops
   repeating that), (b) it consolidates all pre-rebase cleanup (migrated surgical
   resets + new fallback) in one tested place. Cost: one new function + a one-line
   call-site change in `_set_up_worktree()`.

2. **Stash only unexpected residue (committed).** See Spec reconciliation AC2/AC5.

3. **Log stash ref to durable stderr (committed).** See Spec reconciliation AC4.

4. **Scope to the `BEHIND > 0` block (committed).** Cleanup runs only on the rebase
   path. A dirty worktree with `BEHIND == 0` attempts no rebase, so it cannot crash
   here; the pilot-runs-dirty case is the root-cause sibling ticket's surface, not
   this symptom fix. See Scope.

## Files touched

- `skills/bundled/_shared/dispatch-lib.sh` — add `_clean_worktree_for_rebase()`;
  replace lines 301-311 surgical cleanup with a call to it (the helper subsumes and
  extends them).
- `skills/bundled/_shared/test-dispatch-lib.sh` — add `test_resume_dirty_worktree_cleaned()`.
- `docs/plans/2026-06-06-001-fix-dispatch-lib-resume-dirty-worktree-plan.md` — this plan.

## Rollback

Pure dispatch-lib shell change, copy-deployed (not in-binary). Revert the commit and
`make deploy` (or re-sync the bundled skill) to restore prior behavior. No schema,
no migration, no API surface.
