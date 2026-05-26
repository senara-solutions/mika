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

This issue is **fully resolved** by subsequent fixes that shipped after #804 was filed. Both scope items from the issue body are addressed:

- **Original scope:** Add `error_max_turns` as a fourth callback case → resolved by mika#838.
- **Scope amendment** (comment `IC_kwDORWsgGM8AAAABAXbTxg`, 2026-04-25): Amend `On pipeline failure` to use `metadata.retry_pending = true` + `pipeline_retry_count++` instead of forbidden inline `run_claude_pilot` retry → resolved by current handler state (see § 4 below).

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

### 4. Scope amendment — `On pipeline failure` retry mechanism (comment `IC_kwDORWsgGM8AAAABAXbTxg`)

**Current state:** `skills/bundled/self-dev-callback/system_prompt.md` lines 85-90.

The scope amendment requested: *"Amend existing `On pipeline failure` case to use `metadata.retry_pending = true` + `pipeline_retry_count++` instead of the prescribed-but-forbidden inline `run_claude_pilot` retry."*

The current handler satisfies the amendment's **intent** through a functionally equivalent mechanism:

1. **`pipeline_retry_count++` — implemented.** Line 90: `update_task_status` with `metadata: {"pipeline_retry_count": <current + 1>}`. Increment happens before any retry attempt. Gated at `pipeline_retry_count >= 2` for escalation (line 89).

2. **No forbidden inline retry — resolved.** The "forbidden inline `run_claude_pilot` retry" that the amendment cited was the concern that re-dispatching synchronously within the callback turn would bypass the engine's dispatch-slot mechanism. The current handler calls `run_claude_pilot` as a **tool call** (line 90), which enters the engine's long-running task executor. The executor returns `{"status": "deferred", "deferred": true}` when the dispatch slot is occupied — making it engine-mediated, asynchronous, and slot-aware. This is **not** the synchronous inline retry the amendment was forbidding. The handler explicitly notes: "If returns `{"status": "deferred", "deferred": true}`, retry is auto-enqueued — do NOT retry again."

   **Citation:** The engine-mediated, deferred-aware path was introduced in **mika#1058 / PR #1061** (commit `14194d3b`, "fix(executor): callback-safe deferred dispatch for long-running tools"). Prior to #1058, the same `run_claude_pilot` tool call would execute synchronously within the callback turn — which was the forbidden pattern. After #1058, the executor returns `deferred: true` when the dispatch slot is busy, and the heartbeat re-dispatches. The handler text at lines 85-90 is unchanged in form, but its runtime semantics shifted from forbidden-inline to engine-mediated when #1061 merged. Verification: `git log --all --oneline -S "deferred" -- skills/bundled/self-dev-callback/system_prompt.md` shows #1061 as the introducing commit; the handler's "If returns deferred, retry is auto-enqueued" line is the prompt-level reflection of #1061's engine contract.

3. **Regarding `metadata.retry_pending = true`:** The amendment proposed this as the retry signaling mechanism (with the heartbeat #803 handling actual re-dispatch). The current implementation achieves the same outcome differently: `run_claude_pilot` → engine executor → deferred if slot busy. The engine's deferred-dispatch queue (#803) IS the re-dispatch mechanism the amendment envisioned. The `pipeline_retry_count` metadata field serves the same bookkeeping role as `retry_pending` — tracking that a retry is in-flight and capping retries at 2. The implementation chose a simpler path (one tool call that handles both immediate and deferred cases) rather than a two-phase flag-then-heartbeat approach.

**Conclusion:** The amendment's two concerns — (a) stop using a forbidden synchronous retry pattern, and (b) use metadata-tracked retry counts — are both satisfied. The mechanism differs from the amendment's proposed implementation (flag + heartbeat) but achieves the same contract: retries are engine-mediated, capped, and metadata-tracked.

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
| **Scope amendment:** Amend `On pipeline failure` to use `retry_pending` + `pipeline_retry_count++` instead of forbidden inline retry | Lines 85-90: `pipeline_retry_count` incremented in metadata; `run_claude_pilot` is engine-mediated (deferred-aware), not synchronous inline. See § 4 above. | mika#1058 / PR #1061 — engine-side deferred dispatch made the same handler tool call engine-mediated rather than synchronous-inline |

```bash
# Verify error_max_turns recognition exists
grep -c "error_max_turns" skills/bundled/self-dev-callback/system_prompt.md  # -> >= 1
grep -c "recover_unpushed_work" skills/bundled/self-dev-callback/system_prompt.md  # -> >= 1
grep -c "git.*log.*origin/main" skills/bundled/self-dev-callback/system_prompt.md  # -> >= 1

# Verify "On pipeline failure" uses pipeline_retry_count metadata (scope amendment)
grep -c "pipeline_retry_count" skills/bundled/self-dev-callback/system_prompt.md  # -> >= 2 (check + increment)
grep -c "deferred" skills/bundled/self-dev-callback/system_prompt.md  # -> >= 1 (deferred-aware dispatch)

# Verify the engine-mediated dispatch citation (mika#1058 / PR #1061)
git log --all --oneline -S "deferred" -- skills/bundled/self-dev-callback/system_prompt.md | grep -c "callback-safe deferred dispatch"  # -> >= 1

# Verify post-flight push exists in dispatch-lib
grep -c "_push_branch" skills/bundled/_shared/dispatch-lib.sh  # -> >= 1

# Verify compound doc exists
test -f docs/solutions/best-practices/recover-unpushed-claude-pilot-work-2026-04-27.md && echo OK
```

## Recommendation

**Close mika#804 as resolved.** All four layers of the fix are in place, covering both the original scope and the scope amendment (comment `IC_kwDORWsgGM8AAAABAXbTxg`):

1. **Prompt layer (original scope):** `error_max_turns` is recognized as a named verdict class with `recover_unpushed_work` handler (mika#838)
2. **Prompt layer (scope amendment):** `On pipeline failure` handler uses engine-mediated `run_claude_pilot` (deferred-aware, not synchronous inline) with `pipeline_retry_count` metadata tracking — satisfies the amendment's intent to eliminate forbidden inline retries and add metadata-based retry bookkeeping
3. **Infrastructure layer:** dispatch-lib pushes unconditionally in post-flight, preventing the stranded-commits scenario (mika#1268, mika#1282)
4. **Knowledge layer:** Recovery procedure is compounded for institutional memory

No code changes needed. The issue can be closed with a comment citing mika#838, mika#1268, and mika#1282 as the resolving PRs, and noting that the scope amendment's `On pipeline failure` concern is satisfied by the current handler's deferred-aware retry mechanism.

## Files

No files to change. This is a close-as-resolved plan.

## Revision history

- rev 2 (2026-05-26): addressed F1 by verifying the `On pipeline failure` handler's current state against scope amendment `IC_kwDORWsgGM8AAAABAXbTxg`. Added § 4 to resolution audit documenting that `pipeline_retry_count` metadata tracking is implemented (lines 85-90) and `run_claude_pilot` is engine-mediated/deferred-aware (not the forbidden synchronous inline retry the amendment cited). Added scope amendment row to verification table. Updated recommendation to explicitly cover both scope items. Citation: mika#804 comment `IC_kwDORWsgGM8AAAABAXbTxg`; review-guide.md § citation-or-silence.
