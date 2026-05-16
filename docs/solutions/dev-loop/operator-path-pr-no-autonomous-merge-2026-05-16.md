---
title: Operator-path PRs from /mika-spawn don't trigger the autonomous-merge contract
module: dev-loop
date: 2026-05-16
problem_type: workflow_issue
component: autonomous_merge
severity: medium
tags:
  - autonomous-merge
  - dispatch
  - mika-spawn
  - operator-path
  - verdict-handler
  - ci-handler
related_components:
  - mika-dev
  - mika-qa
  - ci_success_handler
  - verdict_handler
  - tasks-table
applies_when:
  - "A PR is opened via a /mika-spawn tactical workflow (operator-path), not via mika-dev autonomous dispatch"
  - "The PR receives mika-qa approval and CI green but does not auto-merge"
  - "Operator wants to understand why the autonomous-merge contract didn't fire"
---

# Operator-path PRs from /mika-spawn don't trigger the autonomous-merge contract

## Symptom

A PR is opened by a tactical `/mika-spawn` workflow (e.g., the orchestrator spawns a tenant for a one-off docs commit, audit cleanup, or small fix). The PR completes the full quality bar:

- All CI checks green.
- mika-qa posts an approving review.
- `pipeline-exempt` label applied (where appropriate).

The autonomous-merge contract does **not** fire. The PR sits with green CI + QA approval indefinitely. Operator eventually admin-merges manually.

## Mechanism

The autonomous-merge contract is a multi-stage handler chain in mika:

```
qa_approval (webhook) → verdict_handler → ci_status (webhook) → ci_success_handler → merge as mika-platform-dev
```

Both handlers key on the existence of a `tasks` row in mika's DB with:

- `branch` matching the PR's head branch, AND
- `status = 'in_progress'`.

When a PR opens, `ci_success_handler` and `verdict_handler` look up the task row. If no row matches, they fall through:

```python
# verdict_handler.rs (paraphrased shape)
task = tasks.find_in_progress_for_branch(pr.head_ref)
if task is None:
    log("no active in_progress task found, passing to LLM")
    forward_to_llm(verdict_event)
    return
```

The LLM (mika-dev's session) correctly identifies "out-of-scope, no task to action" and ends turn. No merge occurs.

**Tactical `/mika-spawn` workflows don't write a `tasks` row.** They are operator-path: a tenant is spawned with an intent, runs to completion, opens a PR, exits. The autonomous loop's task-tracking surface is never engaged. So when the resulting PR gets QA approval, `verdict_handler` cannot find a task and the chain breaks.

## Canonical instance (2026-05-16)

PR #1146 ("docs(solutions): rebase-duplicate plan blob identity pattern") opened via tactical spawn `d0f7e440-…`. Status at merge time:

- 8 green CI checks
- 1 mika-qa review: APPROVED
- 1 `pipeline-exempt` label applied at PR creation time
- 0 rows in `tasks` matching branch `docs/rebase-duplicate-plan-2026-05-16`

`verdict_handler` log line (from server.log, paraphrased): `"branch=docs/rebase-duplicate-plan-2026-05-16 no active in_progress task found, passing to LLM"`. mika-dev's session correctly identified the PR as out-of-scope and ended turn. Operator manually admin-merged ~40 min later.

## Why this matters

The operator-path is increasing in frequency as `/mika-spawn` becomes the default for non-feature work (docs compounds, audit follow-ups, hotfix shims). Each such PR currently requires manual operator merge — a small per-incident cost that compounds across the session. More importantly, it creates a **two-tier merge model** that is undocumented: autonomous-loop PRs merge themselves, operator-path PRs require manual merge, and there is no surfaced explanation of which path a given PR is on.

## Fix options

**Option A — Tactical spawns register a task before opening the PR.**

`/mika-spawn` workflows that intend to open a PR could pre-register a `tasks` row (status `in_progress`, branch set, source `operator-path-spawn`). The autonomous-merge contract then fires as it does for dispatch-path PRs.

Tradeoffs:
- Pollutes the `tasks` table with operator-path rows that have no upstream dispatch.
- Requires every spawn workflow that might open a PR to know about the task surface.
- Couples the spawn primitive to mika's internal data model.

**Option B — Accept "branch-matched-but-no-task" as a merge-eligible state for `pipeline-exempt` PRs.**

`ci_success_handler` and `verdict_handler` could be relaxed: if the PR is labeled `pipeline-exempt`, the absence of a task row is acceptable. Merge anyway when CI + QA pass.

Tradeoffs:
- Requires careful gating (only `pipeline-exempt`, only when QA approves, only when CI green) to avoid merging things the autonomous loop didn't intend.
- The label becomes load-bearing for merge semantics — operator must remember to apply it.
- Cleaner than Option A in that it doesn't pollute the tasks table.

**Option C — Document operator-path as the explicit semantics and standardize admin-merge.**

Accept the two-tier model. Document it: PRs from `/mika-spawn` are operator-path and require operator merge. No system change.

Tradeoffs:
- Cheapest to ship (zero code change).
- Leaves the per-incident manual-merge cost in place.
- Honest about the operator-path role: the operator is on the hook for landing what they spawned.

## Recommended path

**Option C immediately, Option B if frequency justifies the surface.** Option A is over-engineered for the volume.

Document the two-tier model in:
- `mika-skills/self-dev/` (or wherever the spawn-vs-dispatch distinction lives).
- A short callout in `.claude/commands/mika-spawn.md` noting that resulting PRs require operator merge.

Revisit if operator-path PRs exceed ~5/week — at that point Option B's surface starts paying back.

## Detection signal

If you see a PR sitting with:
- Green CI ✓
- mika-qa approval ✓
- No auto-merge after ~10 min

Check whether a `tasks` row exists for the branch:

```sql
SELECT id, status, branch, source
FROM tasks
WHERE branch = '<pr-head-ref>'
ORDER BY created_at DESC LIMIT 5;
```

If no row matches, it's operator-path. Admin-merge is the expected disposition.

## Related

- Memory: `feedback_just_spawn_when_loop_stalls` (operator-path is fast and correct; the trade is the merge tax).
- Adjacent: `docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md` (different failure mode at a different stage of the same contract).
