---
module: task-engine
date: 2026-05-17
problem_type: best_practice
component: background_job
severity: high
related_components:
  - dispatcher
  - audit-events
  - autonomous-loop
tags:
  - task-engine
  - callback
  - structural-backstop
  - coupled-pair
  - autonomous-loop
  - parent-task
  - dispatch-slot
applies_when:
  - "Adding a new engine-level state transition that depends on a fallible downstream actor (LLM, subprocess, network)"
  - "Designing recovery paths for callback handoff failures in the autonomous loop"
  - "Extending the task_engine reaper/promoter/completer family with a new outcome × state combination"
  - "Auditing why an in_progress task stayed wedged across server restarts"
---

# Engine-level coupled-pair structural backstops for callback handoff

> **Pattern in one line:** for every (callback outcome × parent state) combination on the autonomous-loop dispatch path, the task engine must own a structural backstop that fires regardless of whether the silent agent turn calls `update_task_status`. The backstops come in pairs: an inline path at delivery time (fast slot release) and a periodic path (crash-recovery + pre-deploy wedge cleanup). Any change to one sibling's filters must be applied symmetrically to its coupled pair.

## Context

The autonomous-loop dispatch lifecycle (`mika-dev → run_claude_pilot → callback → silent agent → update_task_status`) has multiple steps where the parent task's status transition can fail. Today's terminal states for the parent are `completed` / `failed` / `cancelled` / `expired`; until one of these is reached, the per-class dispatch slot stays occupied and anti-cascade can't release the next deferred dispatch.

Three concrete failure modes have produced production wedges:

| Outcome | Parent state at wedge | Prior fix | Issue |
|---------|----------------------|-----------|-------|
| Callback delivers **without** `pr_url` (subprocess crashed, build failed, dispatch never produced PR) | `in_progress` after grace expired | Reaper marks `failed` | mika#871, refined by #1118, #1126 |
| Callback delivers **with** `pr_url` but reaper already marked `failed` (race after retry) | `failed` | Retry-promoter marks `completed` | mika#958 |
| Callback delivers **with** `pr_url` but silent turn fails to call `update_task_status` (timeout, max-steps continuation drops the call, transport error) | `in_progress` forever | **This pattern** | mika#1162 |

Canonical incident for the third case: mika#1158 on 2026-05-16. Callback ran claude-pilot for 181 turns / $20.82 / ~40 min, delivered cleanly with PR #1160, but the silent agent turn never marked the parent. Parent stuck for 2+ hours; 4 downstream dispatches deferred; manual `mika tasks cancel` required to unwedge.

## Guidance

**Build engine-level backstops as a coverage matrix, not as one-off fixes.** Every dispatch outcome × every parent state combination needs a structural transition path. When you find a wedge that the LLM was *supposed* to resolve via tool call, the answer isn't to harden the prompt; it's to add the engine-level transition that fires regardless.

**Layer each backstop into two paths:**

1. **Inline at delivery time** — a fire-and-forget helper called from `dispatch_resume_agent` *before* `run_silent_agent`. Reads the callback result directly (independent of any prior DB writes the silent agent might depend on). Pattern:

   ```rust
   if is_callback {
       try_extract_callback_metadata(&self.db, task).await;     // mika#376
       try_promote_parent_on_retry_success(&self.db, task).await; // mika#958
       try_complete_parent_on_callback_success(&self.db, task).await; // mika#1162
   }
   ```

   Frees the dispatch slot fast (within the same tick). Each helper has identical scope guards (`trigger_type='manual'`, `source='self_dev'`, `dispatch_class='implement'`) and identical fire-and-forget error handling (warn-log + continue).

2. **Periodic scan in the tick loop** — sibling to the existing reaper at the same cadence (`DB_SCAN_INTERVAL_TICKS`). Catches crash-recovery cases (server died between callback delivery and the inline call) and pre-deploy wedges from before the inline path shipped. Uses a parallel DB query with the same filter shape as the reaper's query, differing only on the predicate that distinguishes the outcome.

**Make the queries mutually exclusive on a single predicate.** The reaper queries `parent.metadata.claude_pilot.pr_url IS NULL`; the completer queries `IS NOT NULL AND != ''`. Same row cannot match both queries. Both run in the same tick, no race.

**Use `update_task_completed`-style guards as the second line of defense.** The DB-level `WHERE status IN ('pending', 'in_progress')` clause makes whichever-fires-first the winner when the inline path, periodic backstop, and agent's own `update_task_status` race. The loser sees `Ok(false)` and logs at debug level — no double-write, no status corruption.

**Emit a single audit-event `tool_name` per coupled-pair.** Both the inline path and periodic backstop in mika#1162 write `task_engine_parent_completer`. The reason string distinguishes source (`parent_completed_from_callback` vs `parent_completed_from_callback_backstop`) so operators can grep one name to see all auto-transitions and dig into the reason for source attribution.

## Why this matters

**The autonomous loop is a chain of fallible actors.** When the LLM is one link in a chain that ends with "and then mark the task complete," every other link is fallible too — including the act of calling the marking tool. Prompt-level enforcement (`callback_terminal_action` INTENT_GUARD #870) keeps the agent honest *when it reaches EndTurn*, but cannot help when the turn times out, hits max-steps without restating the call, or fails before the tool dispatch.

**Wedges in the dispatch slot are catastrophic at scale.** mika#1158 blocked 4 downstream dispatches for 2+ hours. The per-class dispatch slot (mika#1001) limits the agent to one impl + one groom in flight at any time — a single wedge halves throughput. Operator manual recovery (`mika tasks cancel`) is reactive and requires the operator to notice; the structural backstop is proactive.

**Coupled-pair drift is the silent killer.** When a future PR changes the reaper's filters (a new `dispatch_class`, a different grace period, a new scope guard like `source='self_dev_milestone'`), failing to apply the same change to the completer creates split-brain semantics on the success path. The discipline below catches this.

## When to apply

- A new `tasks.status` transition is needed and the existing path depends on an LLM tool call to fire.
- A new callback outcome × parent-state combination is uncovered.
- An incident report shows a task stuck in a non-terminal state where the agent "should have" updated it.
- A new dispatch class is added (per-class slot guards expand): every existing backstop's `dispatch_class` filter needs symmetric update.
- A new `source` flavor is added (e.g., a new dispatch family that isn't `self_dev`): decide explicitly whether each existing backstop applies.

## Examples

### Coupled-pair convention in action

Three sibling functions in `crates/mika-agent/src/task_engine/dispatcher.rs`, all under the same `if is_callback { ... }` block at delivery time:

| Function | Parent state precondition | Callback outcome | Outcome state |
|----------|--------------------------|------------------|---------------|
| `try_extract_callback_metadata` (mika#376) | any | any | metadata write only, no status change |
| `try_promote_parent_on_retry_success` (mika#958) | `status='failed'` | `pr_url` present | `failed → completed` |
| `try_complete_parent_on_callback_success` (mika#1162) | `status='in_progress'` | `pr_url` present | `in_progress → completed` |

Three sibling periodic scans in `crates/mika-agent/src/task_engine/engine.rs`, all in the same tick at the same cadence:

| Function | Parent state predicate | pr_url predicate | Outcome state |
|----------|----------------------|------------------|---------------|
| `reap_orphaned_parent_tasks` (mika#871) | `in_progress` | `IS NULL` | `failed` |
| `complete_parent_tasks_on_callback_success` (mika#1162) | `in_progress` | `IS NOT NULL AND != ''` | `completed` |

The two periodic queries (`find_orphaned_parent_tasks` and `find_completable_parent_tasks_on_pr_url` in `db.rs`) share every filter except the `pr_url` predicate. The doc-comments on both name the coupled pair explicitly.

### Mandatory comment-anchored convention

When adding the completer, the reaper's doc-comment was updated bidirectionally:

```rust
/// Find orphaned parent self_dev tasks whose callback subtask delivered
/// without producing a PR.
/// ...
/// **Coupled pair:** `find_completable_parent_tasks_on_pr_url` is the
/// success-side sibling (mika#1162). Any filter change here (agent_id,
/// status, source, trigger_type, dispatch_class, sibling guard, grace
/// window) MUST be applied symmetrically there. The two queries differ
/// only on the `pr_url` predicate (`IS NULL` here vs `IS NOT NULL` there).
pub fn find_orphaned_parent_tasks(...) { ... }
```

This is the only enforcement we have against drift — there is no compile-time or test-time check that the two queries stay in sync. Future authors editing either site MUST navigate to the other.

### Known limitation (rare cascade)

If `try_extract_callback_metadata` *and* `try_complete_parent_on_callback_success` *both* fail (double DB error) *and* the silent agent fails to call `update_task_status` *and* grace expires, the reaper will fire and mark the parent `failed` because the periodic completer reads from `parent.metadata` (which the failed metadata write never populated). Recovery requires manual operator intervention. Mitigation: the inline path reads `pr_url` from `task.result` directly, independent of metadata write success — so a single DB failure does not produce the cascade. Documented as advisory follow-up; not blocking for typical incident shapes.

## Related Solutions

- **`logic-errors/parent-self-dev-task-leaks-in-progress-after-callback-delivers-2026-04-29.md`** — the failure-side reaper (mika#871, refined by #1118 and #1126). Coupled pair of this fix; any filter change there demands symmetric change here.
- **`logic-errors/dispatch-retry-parent-status-promotion-2026-05-07.md`** — the retry-promoter (mika#958). Third sibling in the inline trinity at `dispatcher.rs:406-411`.
- **`architecture-patterns/engine-level-callback-metadata-extraction.md`** — the shared fire-and-forget pattern (mika#376). All three inline helpers follow this shape.
- **`architecture-patterns/callback-task-loop-prevention.md`** — establishes the SOLE WRITER convention for engine-level status transitions. The auto-completer is the SOLE WRITER of `task_engine_parent_completer` audit events.
- **`959-callback-watchdog-stale-subprocess-detection.md`** — adjacent periodic scanner (`check_callback_process_liveness`). Acts on the child callback; the auto-completer acts on the parent — mutually exclusive populations, no race.
- **`failed-callback-tasks-silently-dropped.md`** (mika#203) — the foundational status-filter audit checklist. Whenever a new scanner is added, run this checklist to verify no existing scanner already picks up the rows you're targeting (or picks them up at the wrong stage).

## Reference

- **Plan:** `docs/plans/2026-05-17-001-fix-1162-task-engine-parent-auto-complete-plan.md`
- **PR:** mika#1162 (this PR)
- **Canonical incident:** mika#1158 (2026-05-16, 2-hour wedge, manually cancelled)
- **Implementation files:**
  - `crates/mika-agent/src/db.rs` — `find_completable_parent_tasks_on_pr_url`, `CompletableParentTask`
  - `crates/mika-agent/src/task_engine/dispatcher.rs` — `try_complete_parent_on_callback_success`
  - `crates/mika-agent/src/task_engine/engine.rs` — `complete_parent_tasks_on_callback_success`
