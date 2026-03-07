---
title: "Team task child tasks created with wrong agent_id causing silent completion failures"
date: 2026-03-07
module:
  - crates/mika-agent/src/teams/engine.rs
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/async_db.rs
  - crates/mika-agent/src/agent.rs
problem_type: logic_error
severity: high
tags:
  - task-engine
  - team-engine
  - agent-id
  - async-database
  - silent-failure
  - query-correctness
  - parent-child-traversal
related_issues: []
---

# Team Task Child Tasks Created with Wrong agent_id

## Problem

When a team run executes, the team engine creates a task tree: a parent `invoke_orchestrator` task plus child `resume_agent` tasks for each delegated agent. All tasks were created with `agent_id = "mika"` (the orchestrator's default), but delegated agents run with their own `AsyncDatabase` bound to their actual name (e.g., `"odds-engine-cto"`). When agents completed, `update_task_completed` filtered `WHERE agent_id = ?` with the agent's own ID — which didn't match the task's stored `"mika"` — so 0 rows were updated. The `Ok(false)` return was silently discarded via `let _ =`.

Additionally, three task tree traversal queries assumed uniform `agent_id` across parent-child relationships, which would break the suspend/resume flow for team runs.

### Symptoms

- Child `resume_agent` tasks permanently stuck in `pending` status
- Recurring `WARN: "task missing next_fire_at"` log spam every 60 seconds (periodic scan picking up orphaned callback tasks)
- No errors visible (silent failure via `let _ =`)

## Root Cause

```
AsyncDatabase::new() defaults agent_id to "mika"
    └─ run_team tool uses AsyncDatabase::new(db) for team_db
        └─ execute_tasks() creates child tasks with self.team_db.agent_id = "mika"
            └─ Agent runs with resources.db (new_with_agent(db, &ta.name)) = correct ID
                └─ update_task_completed: WHERE id = ? AND agent_id = "odds-engine-cto"
                    └─ Task has agent_id = "mika" → 0 rows updated → Ok(false)
                        └─ let _ = discards the false → silent failure
```

Compounding factors:
1. `try_complete_parent_on_sibling_done` filtered sibling count and parent claim by `agent_id` — would count 0 siblings from other agents and fail to claim parent with different `agent_id`
2. `get_child_tasks` filtered by `agent_id` — orchestrator would see 0 children when loading results for team resume
3. `get_expired_child_task_ids` JOIN required `p.agent_id = t.agent_id` — expired children invisible when parent has different `agent_id`

## Solution

### 1. Fix child task creation (teams/engine.rs:743)

```rust
// Before:
agent_id: self.team_db.agent_id.clone(),  // always "mika"

// After:
agent_id: input.agent_name.clone(),  // actual agent name
```

### 2. Remove agent_id from task tree traversal queries (db.rs)

Parent-child relationships are structural (via `parent_task_id`), not scoped by `agent_id`. Three methods updated:

**`try_complete_parent_on_sibling_done`** — removed `agent_id` from parent lookup, sibling count, and parent claim queries.

**`get_child_tasks`** — removed `agent_id` filter, queries by `parent_task_id` only.

**`get_expired_child_task_ids`** — removed `p.agent_id = t.agent_id` JOIN condition.

**`count_pending_callback_tasks_by_team_run`** — removed `agent_id` parameter, queries by `team_run_id` only.

### 3. Exclude callback tasks from scheduler scan (db.rs)

```rust
// get_schedulable_tasks — added filter:
AND trigger_type != 'callback'
```

Callback tasks are event-driven (completed externally), not scheduled. Without this filter, the tick loop repeatedly tried to enqueue them, hitting the "task missing next_fire_at" warning.

### 4. Replace silent error discards (agent.rs + 14 more sites)

```rust
// Before:
let _ = db.update_task_completed(task_id, Some(&text)).await;

// After:
match db.update_task_completed(task_id, Some(&text)).await {
    Ok(false) => warn!(task_id, "child task completion had no effect"),
    Err(e) => warn!(task_id, error = %e, "failed to complete child task"),
    Ok(true) => {}
}
```

Applied across `engine.rs`, `dispatcher.rs`, `executor.rs`, and `handlers.rs`.

## Prevention

### Decision Framework: agent_id vs parent_task_id

| Query Intent | Filter By | Rationale |
|---|---|---|
| "My tasks" (TUI, dashboard) | `agent_id` | Single-agent scope |
| "Are all siblings done?" | `parent_task_id` only | Siblings span agents |
| "Count pending callbacks for team" | `team_run_id` only | Team spans agents |
| "Dispatch parent after children" | `parent_task_id` only | Parent is different agent |

**Rule:** If the query traverses `parent_task_id` or `team_run_id`, it must NOT filter by `agent_id`.

### Testing Pattern

Always test task trees with heterogeneous agent_ids:

```rust
let parent = create_task(&db, agent_id: "orchestrator", ...);
let child_a = create_task(&db, agent_id: "researcher", parent: parent.id, ...);
let child_b = create_task(&db, agent_id: "writer", parent: parent.id, ...);
// Verify cross-agent traversal works
```

### Code Review Checklist

- Does this DB mutation's `Result` get checked? `let _ =` on mutations is a defect.
- Does this query cross a `parent_task_id` boundary? If yes, no `agent_id` filter.
- Does the test use multiple distinct `agent_id` values?

## Related Documentation

- [Consolidate per-agent DBs](consolidate-per-agent-team-dbs-into-single-container-db.md) — `AsyncDatabase::new_with_agent()` pattern
- [Callback resume lifecycle](../architecture/callback-resume-agent-lifecycle.md) — TOCTOU-safe `update_task_completed`
- [Callback TUI delivery](../architecture-patterns/callback-tui-delivery-polling.md) — polling and delivery mechanism
- [Callback loop prevention](../architecture-patterns/callback-task-loop-prevention.md) — orchestrator guards
- [Code review findings 522-542](../code-review-patterns/async-callbacks-long-running-review-findings.md) — related task engine issues
