---
title: "fix(self-dev): callback handler has no error_max_turns case — already resolved"
type: fix
status: resolved
date: 2026-05-26
origin: senara-solutions/mika#804
---

# Plan — self-dev callback handler has no error_max_turns case (mika#804)

**Issue:** [mika#804](https://github.com/senara-solutions/mika/issues/804)
**Type:** fix (self-dev callback handler gap)
**Labels:** bug, p2-normal, agent-core, skill
**Status:** Already resolved — recommend closing

## Problem (per issue body)

The self-dev callback handler recognized only three callback shapes (pipeline failure, success, generic failure). The `error_max_turns` callback (`[guardrail] error_max_turns: SDK limit reached after 201 turns`) didn't match any cleanly, causing partial-pipeline runs to be unrecoverable. The founding incident was mika#798 (2026-04-25).

The issue proposed adding a fourth explicit case with `pipeline_retry_count` based retry logic, gated on #803 (engine retry primitive) landing first.

## Resolution audit — all gaps closed by subsequent work

This issue is **fully resolved** by two subsequent fixes that shipped after #804 was filed:

### 1. mika#838 — `error_max_turns` recognition + `recover_unpushed_work` verdict

**Shipped in:** `skills/bundled/self-dev-callback/system_prompt.md` (lines 52-83, current)

Added the "Pipeline result classification" section with:

- **Primary trigger:** `tasks.result` contains literal `error_max_turns` substring (line 54)
- **Secondary trigger:** Conservative heuristic for stale in-progress tasks with NULL result, no PR, and created > 2 hours ago (lines 56-61)
- **Grounding check:** `git -C <repo-path> log --oneline origin/main..<branch>` — queries local branch state before declaring no work exists (lines 63-70)
- **Decision tree:** >= 1 commit -> `recover_unpushed_work` verdict; 0 commits -> fall through to "On failure" (lines 72-75)
- **Handler:** Writes `unpushed_recovery_pending: true` to metadata, emits structured `send_message` to operator with branch/sha/commit-count/recovery-command, does NOT increment `pipeline_retry_count`, does NOT redispatch (lines 77-83)

This took a **better approach** than #804's proposed retry logic: instead of retrying (which re-does already-completed work), it detects unpushed commits and routes to operator-assisted recovery. The `recover_unpushed_work` verdict class is also cross-referenced in `self-dev-webhook-qa/system_prompt.md` for taxonomy completeness.

### 2. mika#1268 + mika#1282 — unconditional post-flight push + dirty-worktree rescue

**Shipped in:** `skills/bundled/_shared/dispatch-lib.sh`

- **`_push_branch()`** (line 655): Runs after `_run_claude_pilot()` regardless of pilot exit code. Pushes any local-ahead commits to origin. Handles first-push and already-in-sync cases.
- **Dirty-worktree rescue** (lines 417-490, mika#1282): When dev-pilot exits 0 but HEAD unchanged, detects dirty files, auto-commits with `wip()` prefix, and opens a draft PR for operator review.

Together these make the `error_max_turns` scenario a **non-issue at the infrastructure layer**: even if claude-pilot hits the turn limit before pushing, dispatch-lib pushes unconditionally in post-flight. The `recover_unpushed_work` prompt-level handler remains as defense-in-depth for edge cases where the push itself fails.

### 3. mika#803 (dependency) — closed

The engine retry primitive (#803) that #804 listed as a blocker has shipped and is closed.

### Compounded knowledge

The recovery pattern is documented at `docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md`, which includes the failure shape, grounding rule, decision tree, recovery procedure, and explicit out-of-scope guardrails. The doc's `resolved_by` field already cites mika#1268.

## Verification

All acceptance criteria from #804's proposed solution are satisfied or superseded:

| #804 proposed | Current state | Satisfied by |
|---|---|---|
| Recognize `error_max_turns` as distinct callback class | `self-dev-callback/system_prompt.md` lines 52-54 (primary trigger) | mika#838 |
| Check `gh pr list --head <branch>` for existing PR | Lines 58-59 (secondary trigger), plus line 92 (success path) | mika#838 |
| Retry with `pipeline_retry_count` (max 2) if no PR | **Superseded** — `recover_unpushed_work` is better than retry (doesn't re-do completed work) | mika#838 (design decision) |
| Escalate after 2 retries | Covered by existing "On pipeline failure" handler (lines 85-90) for genuine failures; `recover_unpushed_work` routes to operator directly | mika#838 |
| Depends on #803 engine retry primitive | #803 is CLOSED | mika#803 |

```bash
# Verify error_max_turns recognition exists
grep -c "error_max_turns" skills/bundled/self-dev-callback/system_prompt.md  # -> >= 1
grep -c "recover_unpushed_work" skills/bundled/self-dev-callback/system_prompt.md  # -> >= 1
grep -c "git.*log.*origin/main" skills/bundled/self-dev-callback/system_prompt.md  # -> >= 1

# Verify post-flight push exists in dispatch-lib
grep -c "_push_branch" skills/bundled/_shared/dispatch-lib.sh  # -> >= 1

# Verify compound doc exists
test -f docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md && echo OK
```

## Recommendation

**Close mika#804 as resolved.** All three layers of the fix are in place:
1. **Prompt layer:** `error_max_turns` is recognized as a named verdict class with appropriate handler
2. **Infrastructure layer:** dispatch-lib pushes unconditionally in post-flight, preventing the stranded-commits scenario
3. **Knowledge layer:** Recovery procedure is compounded for institutional memory

No code changes needed. The issue can be closed with a comment citing mika#838, mika#1268, and mika#1282 as the resolving PRs.

## Files

No files to change. This is a close-as-resolved plan.
