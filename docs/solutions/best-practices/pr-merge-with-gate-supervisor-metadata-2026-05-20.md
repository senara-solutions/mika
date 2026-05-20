---
module: tools
tags: [pr-merge-with-gate, supervisor-metadata, reaper, parent-completer, callback-task, claude-pilot, autonomous-loop, structural-enforcement]
problem_type: false-failure-state
category: best-practices
---

# Tools that establish positive PR-existence MUST write `$.claude_pilot.pr_url` to the supervisor

## Problem (mika#1211)

When `pr_merge_with_gate` returned `auto_merge_enabled` (CI pending, GitHub auto-merge queued), it did not record the PR URL on the manual-parent supervisor's metadata. If `try_extract_callback_metadata` had also missed populating `$.claude_pilot.pr_url` from the dispatch-lib `PR:` line (race with later callbacks, `gh pr list --head` empty, qa-only callback delivery), the supervisor sat `in_progress` with `pr_url IS NULL`. The orphan reaper (`reap_orphaned_parent_tasks`, mika#871) then matched the supervisor on its next tick — `parent.status='in_progress' AND parent.source='self_dev' AND parent.trigger_type='manual' AND child.status='delivered' AND child.updated_at < now-600s AND pr_url IS NULL AND no active siblings` — and flipped it to `failed` with `tasks.result='callback_delivered_without_pr_url'`, **without any tool call**, even though the PR was demonstrably queued and merged later.

mika#1204 reproduced the failure on PR #1206: auto-merge enabled at 09:41:46Z, `update_task_status` flipped the supervisor `blocked → in_progress` at 09:41:54Z, reaper flipped it `in_progress → failed` at 09:42:09Z (15s later, consistent with the reaper's 60s tick cadence + child delivered outside the 600s grace). PR #1206 auto-merged cleanly later; the supervisor stayed `failed`.

## Principle

Any tool that establishes a deterministic positive "a PR exists at this URL" signal MUST write `$.claude_pilot.pr_url` to the supervisor's metadata at the moment of decision. The reaper's `pr_url IS NULL` predicate and the parent-completer's `pr_url IS NOT NULL` predicate (mika#1162) partition the world on this exact key; any code path that knows a PR exists is responsible for keeping the partition accurate.

This is the same structural reason `try_extract_callback_metadata` (mika#376) writes pr_url from dispatch-lib's `PR:` line — pr_url is the canonical "a PR was produced" fact. The fix extends the principle from "callback-result-text parsing" to "tool-output-driven writes."

## Pattern

Mirror `dispatcher::try_extract_callback_metadata`:

1. **Resolve supervisor via callback context** — `ToolContext.callback_task_id → callback.parent_task_id`. Skip silently if no callback context (conversation mode, mika-arch invocation).
2. **Gate on the reaper's filter set** — only write when `parent.trigger_type == "manual"` AND `parent.source == Some("self_dev")`. Skip milestone/project parents, chained callbacks, operator tasks.
3. **Two-level shallow merge** via `task_metadata::merge_metadata` so `claude_pilot.pr_url` co-exists with `cost_usd`, `session_id`, `turns`, etc. (mika#489 lineage).
4. **Fire-and-forget persist** — log errors, never propagate into the tool result. The tool's primary contract is "did auto-merge enable?" — metadata-write failure must not surface as `gate_errored`.

## Coupled pair

- **Writers of `$.claude_pilot.pr_url`:** `dispatcher::try_extract_callback_metadata` (callback-result `PR:` line, mika#376) + `tools::pr_merge_with_gate` (`auto_merge_enabled` branch, mika#1211).
- **Readers of `$.claude_pilot.pr_url`:** `db::find_orphaned_parent_tasks` (negative predicate — null means reapable, mika#871) + `db::find_completable_parent_tasks_on_pr_url` (positive predicate — non-null means promotable, mika#1162).

Future changes to the JSON path, the merge semantics, or the reaper/completer predicates MUST consider all four sites. Symmetric extension to `MergeGateResult::Merged` and `::AlreadyMerged` is a clean follow-up if forensics surface supervisors leaking in those branches too.

## Why not fix at the reaper

The reaper's contract — "callback delivered without pr_url means the supervisor's work didn't produce a PR" — is correct. Heuristics like "recently transitioned" or "note matches /auto.?merge/" would couple the engine to mika-dev's free-text prompt, which drifts across model swaps. Recording pr_url at the semantic source (the tool that confirmed the PR exists) cures the predicate without weakening the reaper's contract.

## Why not fix at the prompt

`update_task_status` already includes the auto-merge fact in a free-text `note`. Tightening that to a structured `auto_merge_pending` metadata key would couple the engine to prompt discipline; the tool-level write is structural and fires regardless of what mika-dev writes.

## References

- mika#1211 — this fix
- mika#871, mika#1126 — orphan reaper
- mika#1162 — parent-completer
- mika#376 — `try_extract_callback_metadata` (mirror writer)
- mika#489 — two-level shallow metadata merge (`task_metadata::merge_metadata`)
- `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md` — the precedent pattern
- `docs/solutions/architecture-patterns/work-item-metadata-two-level-shallow-merge.md` — merge semantics
