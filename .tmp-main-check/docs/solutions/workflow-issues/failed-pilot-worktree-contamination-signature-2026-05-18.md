---
title: "Failed mika-dev pilot leaves meta-repo dispatcher content in sub-repo worktree — recovery signature"
date: 2026-05-18
last_updated: 2026-06-06
category: workflow-issues
module: dev-pilot
problem_type: workflow_issue
component: autonomous-loop
severity: medium
applies_when:
  - Operator-spawn recovery of a `pipeline-incomplete-empty-handed` mika-dev pilot failure
  - A target-repo worktree shows uncommitted modifications to `.claude/commands/mika.md` plus untracked meta-repo command files (`mika-spawn.md`, `mika-handsoff.md`, etc.)
  - The branch has diverged from origin with the same plan commit message under a different SHA
  - Third+ failure in the same `#1168`-family pattern
tags:
  - mika-dev
  - autonomous-loop
  - worktree-hygiene
  - pipeline-incomplete-empty-handed
  - recovery-procedure
  - mika-1168
---

# Failed pilot worktree contamination signature

## Context

> **Root-cause correction (mika#1415, 2026-06-06).** The original attribution below —
> "the pilot confuses which repo it is operating in" — was disproven by a Phase 0 pin.
> The signature (`.claude/commands/mika.md` modified + untracked meta siblings) is **not**
> pilot confusion; it is **mechanical and pilot-independent**: `dispatch-lib`'s worktree
> setup ran a blanket `cp -r "$PLATFORM_DIR/.claude/commands" "$WORKTREE_DIR/.claude/"` at
> every dispatch, clobbering the sub-repo's own tracked `/mika` and dropping the meta
> siblings. mika#1415 removes the cause; after it ships the contamination is no longer
> regenerated. The recovery procedure below stays valid for worktrees created before that
> fix. See `docs/solutions/architecture-patterns/seed-scaffold-into-tracked-worktree-dir-via-git-exclude.md`.

mika-dev's autonomous loop occasionally produces a `pipeline-incomplete-empty-handed` failure (mika#1168 family) — many turns burned, zero commits, no PR. A subset of these failures leaves the target-repo worktree in a recognizable contaminated state. The original 2026-05-18 write-up attributed this to the pilot confusing *which repo it is operating in* and editing meta-repo dispatcher files; the mika#1415 pin (above) showed the real cause is dispatch-lib's command-seed `cp -r`, which contaminates the worktree regardless of what the pilot does. The third incident on this branch family (mika#1189 recovery, 2026-05-18) made the signature explicit enough to document.

The contamination is hard to spot at first glance because the bad files look plausible: they're real, well-formed meta-repo command files, just dropped in the wrong place. A surface-level review of `git status` shows "modifications to slash commands" and the operator may not immediately recognize that the pilot was supposed to be editing `crates/...`, not `.claude/commands/`.

## Recovery procedure

When inheriting a worktree from a failed pilot, look for **all three** of these signals together. Any single signal in isolation is normal worktree state; the combination is the contamination signature:

1. **Modified files** include `.claude/commands/mika.md` (the target repo's own `/mika` definition).
2. **Untracked files** include `mika-spawn.md`, `mika-handsoff.md`, `mika-onboarding.md`, `mika-groom-ticket.md`, or similar meta-repo command files — files that **should not exist** in a sub-repo's `.claude/commands/` directory.
3. **Branch is diverged** from origin: same plan commit message present locally and on origin, but with different SHAs (the pilot rebased onto a stale base and produced a duplicate commit).

Verify the diagnosis explicitly before any destructive action — the operator-spawn-recovery convention requires it:

```bash
# 1. Confirm the modified-files / untracked-files signature
git status --short

# 2. Confirm the duplicate-plan-commit signature
git log --oneline main..HEAD
# If you see TWO commits with messages matching the plan (one yours, one already on origin's main),
# the local commit is the pilot's stale rebase. Verify content equality:
LOCAL_PLAN=$(git rev-parse HEAD)
ORIGIN_PLAN=$(git log origin/<branch> --oneline -1 | awk '{print $1}')
git show --stat "$LOCAL_PLAN" | head -3
git show --stat "$ORIGIN_PLAN" | head -3
# Same author + same date + same message + (typically) same single file change = stale rebase.
```

Once verified, the cleanup is deterministic. **Confirm with the operator before running** — these are destructive operations:

```bash
# Reset to the authoritative origin state. Drops the duplicate local commit
# (which was a stale-base rebase of the same plan).
git fetch origin <branch>
git reset --hard origin/<branch>

# Remove the contaminated untracked meta-repo command files. The .claude/commands/
# directory is the only place this contamination class lives — limit the rm
# scope accordingly so unrelated untracked files don't get caught.
git ls-files --others --exclude-standard .claude/commands/ | while read -r f; do
    rm -v "$f"
done

# Confirm clean
git status
```

Expected end state: working tree clean, branch matches `origin/<branch>` exactly, and the architect-committed plan file (committed via `/mika-groom-ticket`) is present at the expected path.

## Why This Matters

The autonomous-loop has now produced three failures in this family (mika#1168, then mika#1189 attempt 1, then mika#1189 attempt 2 as a different shape). Each recovery currently costs the operator 5–15 minutes of diagnostic work — `git status`, `git diff`, `git log` cross-referencing, then deciding whether the untracked files are intentional. A documented signature collapses that to a 30-second check against the three-signal pattern.

The contamination is **not** harmful in itself (no production data touched, no secrets leaked) but it blocks the recovery worker until cleaned: the pipeline pre-commit hook (`.claude/hooks/check-mika-pipeline.sh`) blocks source edits when the worktree's modified files aren't in a worktree path, and the contaminated `.claude/commands/mika.md` confuses any subsequent `/mika` invocation by replacing the target repo's own dispatch logic with the meta-repo's.

The duplicate-commit-with-different-SHA signal is independently load-bearing for two reasons:

1. It tells you a stale rebase happened — likely because the pilot's worktree branched from a `local main` that was behind `origin/main`, so any commits ended up on the wrong base. The clean fix is `reset --hard origin/<branch>`, not a manual merge.
2. It distinguishes "pilot tried and failed" from "operator started recovery, made progress, then handed back" — the latter would show a forward-only branch, not divergence.

## When to Apply

Apply this recovery procedure when:

- An operator-spawn tenant is invoked for `<repo>#<N>` recovery after the prior autonomous dispatch reported `pipeline-incomplete-empty-handed`
- The target worktree at `.claude/worktrees/<branch>/<repo>/` exists from a prior dispatch
- `git status` shows the three-signal combination above

Do **not** apply when:

- The pilot reported a *different* failure mode (e.g., `pipeline-failed`, `gate-errored`, `qa-rejected`) — those have their own state shapes and reset semantics differ.
- The worktree is fresh / unused — the failure happened before any work hit disk. Just dispatch normally.
- Only one of the three signals is present — that's normal in-progress state, not contamination.

## Examples

**Real diagnosis (mika#1189 recovery, 2026-05-18):**

```bash
$ git status --short
 M .claude/commands/mika-issue.md
 M .claude/commands/mika-issues.md
 M .claude/commands/mika.md
?? .claude/commands/mika-ask-a-friend.md     # ← meta-repo file, wrong place
?? .claude/commands/mika-handsoff.md         # ← meta-repo file, wrong place
?? .claude/commands/mika-spawn.md            # ← meta-repo file, wrong place
# (+ 13 more meta-repo command files as untracked)

$ git log --oneline main..HEAD
5297f6ca docs(plans): groom mika#1189 initial plan (operator-directed resolution post-arch ESCALATE)
e70b8ae1 docs(plans): groomed plan for tier1.py expansion (mika#1191 Phase A) (#1194)

$ git log --oneline origin/feat/1189/mika-gateway-bidirectional-sse-channel -1
a1e6974e docs(plans): groom mika#1189 initial plan (operator-directed resolution post-arch ESCALATE)

# Local 5297f6ca and origin a1e6974e share the same message — different SHA.
# The contaminated mika.md diff replaced sub-repo content with meta-repo dispatcher content (the smoking gun).
```

After running the cleanup, the worktree returned to a clean `origin/feat/1189/...` state at commit `a1e6974e`, and the recovery pipeline could proceed normally.

## Related

- `docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md` — adjacent failure mode (grooming dispatch exits without committing the plan); recovery is different but the same family of empty-handed mika-dev failures
- `docs/solutions/architecture/parallel-fork-dispatch-pattern-2026-05-13.md` — operator-spawn parallel-fork pattern (which is what produced this recovery session)
- `.claude/hooks/check-mika-pipeline.sh` — the gate that catches source-code edits outside worktrees; informative when assessing whether contamination is benign or actively blocking
- mika#1168 — original `pipeline-incomplete-empty-handed` failure class
- mika#1189 review run — `.context/compound-engineering/ce-review/20260518-110934-b1793531/` (this session)
