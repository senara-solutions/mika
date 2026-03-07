---
title: "Fix team task agent_id mismatch causing orphaned pending tasks"
type: fix
status: active
date: 2026-03-07
---

# Fix team task agent_id mismatch causing orphaned pending tasks

## Overview

When a team run executes, the team engine creates a task tree (parent `invoke_orchestrator` + child `resume_agent` tasks) all with `agent_id = "mika"`. When team agents complete, they call `update_task_completed` using their own `AsyncDatabase` which has their agent-specific ID (e.g., `"odds-engine-cto"`). The SQL `WHERE agent_id = ?` clause doesn't match, so child tasks are never marked completed — they remain permanently "pending". This also causes log spam from the periodic task scan.

## Problem Statement

Three related issues discovered during integration testing:

### Bug 1: Agent ID mismatch prevents child task completion (Critical)

**Root cause chain:**

1. `run_team` tool opens DB with `AsyncDatabase::new(db)` which defaults to `agent_id = "mika"` (`async_db.rs:40-41`)
2. `execute_tasks()` creates all tasks (parent + children) with `self.team_db.agent_id` = `"mika"` (`teams/engine.rs:713, 743`)
3. Each agent runs with `params.db = &resources.db` where `resources.db` has the agent's own ID (`teams/engine.rs:819`)
4. On completion, `run_team_agent` calls `params.db.update_task_completed(task_id, ...)` (`agent.rs:1630-1635`)
5. `update_task_completed` SQL: `WHERE id = ?2 AND agent_id = ?3` (`db.rs:977`) — agent_id doesn't match, 0 rows updated
6. Return value `Ok(false)` is silently discarded with `let _ =` (`agent.rs:1632`)

**Evidence:** Database shows 2 child tasks (`team-agent-odds-engine-cto`, `team-agent-odds-engine-quant`) stuck in `pending` status despite both agents completing successfully.

### Bug 2: Orphaned pending tasks cause log spam (Minor)

The permanently-pending child tasks have `next_fire_at = NULL`. Every 60 ticks, `scan_db_for_new_tasks()` picks them up via `get_schedulable_tasks()` (which returns `pending` tasks), then `enqueue_queued_task()` warns "task missing next_fire_at" and drops them. This repeats indefinitely.

**Evidence:** Both odds-engine-cto and odds-engine-quant logs show recurring `WARN: "task missing next_fire_at"` every 60 seconds for the orphaned task IDs.

### Design clarification: Parent task cancelled (Expected)

The parent `invoke_orchestrator` task being `cancelled` is correct — when a team run completes synchronously (no suspension needed), the parent is explicitly cancelled at `teams/engine.rs:997` to prevent the async resume path from racing with the synchronous flow.

## Proposed Solution

**Fix the agent_id on child tasks** so they match the agent that will complete them. This is the cleanest fix because:
- Tasks conceptually belong to the agent executing them
- It makes the task table more queryable/debuggable (you can see which agent owns which task)
- The parent `invoke_orchestrator` stays as `"mika"` (correct — mika is the orchestrator)

### Changes

#### 1. `crates/mika-agent/src/teams/engine.rs` — Use agent name for child task agent_id

In `execute_tasks()`, change child task creation (around line 743) to use the actual agent name instead of `self.team_db.agent_id`:

```rust
// Before (line 743):
agent_id: self.team_db.agent_id.clone(),

// After:
agent_id: agent_name.clone(),
```

#### 2. `crates/mika-agent/src/teams/engine.rs` — Pass team_db for child completion

The child task now has `agent_id = "odds-engine-cto"`, but `resources.db` also has `agent_id = "odds-engine-cto"`. So `update_task_completed` will match. No change needed in `agent.rs` — the existing code will work correctly once the task's `agent_id` matches the agent's DB `agent_id`.

**Wait — verify this:** `resources.db` is the per-agent `AsyncDatabase`. Confirm its `agent_id` matches the agent name.

#### 3. `crates/mika-agent/src/teams/engine.rs` — Update grandchild detection query

`count_pending_callback_tasks_by_team_run` in `db.rs:1232` filters by `agent_id`. After the fix, child tasks will have different agent_ids, so grandchild tasks (created by those agents) will also have agent-specific IDs. The query must be updated to not filter by `agent_id` when counting by `team_run_id`:

```rust
// Before (db.rs:1237-1241):
"SELECT COUNT(*) FROM tasks
 WHERE agent_id = ?1
   AND team_run_id = ?2
   AND trigger_type = 'callback'
   AND status = 'pending'
   AND depth > 1",
params![agent_id, team_run_id],

// After:
"SELECT COUNT(*) FROM tasks
 WHERE team_run_id = ?1
   AND trigger_type = 'callback'
   AND status = 'pending'
   AND depth > 1",
params![team_run_id],
```

Update the function signature to remove the `agent_id` parameter, and update the async wrapper and all callers.

#### 4. `crates/mika-agent/src/task_engine/engine.rs` — Filter out callback tasks in scan

`get_schedulable_tasks` should exclude `trigger_type = 'callback'` tasks (they're event-driven, not schedule-driven):

```rust
// Before (db.rs:933-936):
"SELECT {} FROM tasks
 WHERE agent_id = ?1 AND status IN ('pending','recurring_active')
 ORDER BY next_fire_at ASC NULLS LAST",

// After:
"SELECT {} FROM tasks
 WHERE agent_id = ?1 AND status IN ('pending','recurring_active')
   AND trigger_type != 'callback'
 ORDER BY next_fire_at ASC NULLS LAST",
```

This eliminates the log spam for callback tasks that intentionally have no schedule.

#### 5. `crates/mika-agent/src/agent.rs` — Log warning on failed child task completion

Replace `let _ =` with actual error logging so future mismatches are caught:

```rust
// Before (agent.rs:1630-1635):
if let Some(task_id) = params.child_task_id {
    let result_text = result.text.as_deref().unwrap_or("");
    let _ = params
        .db
        .update_task_completed(task_id, Some(result_text))
        .await;
}

// After:
if let Some(task_id) = params.child_task_id {
    let result_text = result.text.as_deref().unwrap_or("");
    match params.db.update_task_completed(task_id, Some(result_text)).await {
        Ok(false) => warn!(task_id, "child task completion had no effect (already completed or agent_id mismatch)"),
        Err(e) => warn!(task_id, error = %e, "failed to complete child task"),
        Ok(true) => {}
    }
}
```

Apply the same pattern to the other `let _ =` at line 1623.

#### 6. Manual DB cleanup — Fix existing orphaned tasks

```sql
UPDATE tasks SET status = 'cancelled', updated_at = unixepoch()
WHERE parent_task_id IS NOT NULL
  AND action_type = 'resume_agent'
  AND status = 'pending'
  AND team_run_id IS NOT NULL;
```

## System-Wide Impact

- **`try_complete_parent_on_sibling_done()`**: Currently checks sibling completion using `agent_id` scoping. With per-agent child task IDs, the parent lookup via `parent_task_id` still works (it queries by parent ID, not agent_id). No change needed.
- **`check_expired_siblings()`**: Same — queries by parent relationship, not agent_id. Safe.
- **Team suspend/resume path**: `count_pending_callback_tasks_by_team_run` is the key query — fix #3 addresses this.
- **TUI callback polling**: `get_undelivered_callback_tasks` filters by agent_id — team child tasks were never picked up by TUI (they use `resume_agent` action, not the TUI delivery path). No impact.

## Acceptance Criteria

- [x] Child `resume_agent` tasks are created with the executing agent's name as `agent_id`
- [x] Child tasks transition to `completed` status when agents finish
- [x] Parent `invoke_orchestrator` is cancelled on sync completion (unchanged behavior)
- [x] No "task missing next_fire_at" warnings for callback-type tasks in logs
- [x] `let _ =` on `update_task_completed` replaced with warning logs
- [x] Grandchild detection query works without agent_id filter
- [x] Existing orphaned tasks cleaned up
- [x] All existing tests pass (`cargo test`)
- [ ] New test: child task completion in team run verifies status transition

## Sources

- `crates/mika-agent/src/teams/engine.rs:712-772` — task tree creation
- `crates/mika-agent/src/agent.rs:1622-1636` — child task completion
- `crates/mika-agent/src/async_db.rs:40-41` — default agent_id "mika"
- `crates/mika-agent/src/async_db.rs:219-224` — update_task_completed wrapper
- `crates/mika-agent/src/db.rs:968-981` — update_task_completed SQL with agent_id filter
- `crates/mika-agent/src/db.rs:932-944` — get_schedulable_tasks (returns callback tasks)
- `crates/mika-agent/src/db.rs:1232-1248` — count_pending_callback_tasks_by_team_run
- `crates/mika-agent/src/task_engine/engine.rs:326-333` — "task missing next_fire_at" warning
- `crates/mika-agent/src/tools/run_team.rs:88-93` — team_db initialization
