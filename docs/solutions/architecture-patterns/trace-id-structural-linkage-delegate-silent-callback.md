---
title: "Strengthen trace_id Structural Linkage Across Delegate, Silent, and Callback Boundaries"
category: architecture-patterns
severity: medium
date: 2026-03-15
tags: [trace-id, observability, team-engine, task-engine, schema-migration, unified-timeline]
related_issues: ["#161"]
related_docs:
  - docs/solutions/architecture-patterns/trace-id-correlation-unified-observability.md
  - docs/solutions/database-issues/trace-id-observability-gaps-callback-team-timeline.md
  - docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md
---

## Problem

Three structural gaps broke end-to-end trace correlation:

1. **Delegate agents in team runs** generated fresh `trace_id` values at `run_team_agent_inner_impl()` instead of reusing the `TeamEngine`'s trace_id. Result: delegate tool calls, audit events, and messages were invisible when querying by the team's trace_id.

2. **Silent agent execution** (heartbeat, callback, reflection, skill_run) generated fresh `trace_id` at `run_silent_inner()`, and the `tasks` table had no `execution_trace_id` column. Result: you could find what created a task but not what happened when it ran.

3. **Callback sessions** (`callback-{uuid}`) had no `parent_session_id` reference. The task's `created_by_session` captured the originating session at creation time, but the new session created by `dispatch_resume_agent` was orphaned. Result: session chain traversal was broken.

## Root Cause

The `TeamAgentParams` and `SilentAgentParams` structs had no `trace_id` field. Both `run_team_agent_inner_impl()` and `run_silent_inner()` always called `generate_trace_id()` — severing the correlation chain at every async boundary (team delegation, task dispatch).

The `tasks` table only had `created_trace_id` (trace context when task was created) with no field for the execution trace. The `sessions` table had no parent linkage column.

## Solution

### Schema v11 Migration

```sql
ALTER TABLE tasks ADD COLUMN execution_trace_id TEXT;
ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;
CREATE INDEX IF NOT EXISTS idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;
```

Updated `unified_timeline` VIEW task leg:
```sql
COALESCE(execution_trace_id, created_trace_id) AS trace_id
```

### Trace_id Propagation Pattern

Added `trace_id: Option<String>` to both `TeamAgentParams` and `SilentAgentParams`. In the respective `_inner_impl` functions:

```rust
let trace_id = params
    .trace_id
    .clone()
    .unwrap_or_else(mika_common::trace::generate_trace_id);
```

This preserves backward compatibility — callers that pass `None` still get a fresh trace_id.

### Callers Updated

- **Team engine** (`execute_tasks`, `run_single_agent_task`): passes `self.trace_id.clone()` into `TeamAgentParams.trace_id`
- **delegate_task tool**: passes `ctx.trace_id.to_string()` into `TeamAgentParams.trace_id`
- **All 4 dispatcher methods** (heartbeat, reflection, callback, skill_run): generate trace_id, pass to `SilentAgentParams.trace_id`, write back via `update_task_execution_trace_id` after the run
- **Callback and skill_run dispatchers**: switched to `create_session_with_parent` with `task.created_by_session` as the parent
- **Heartbeat and reflection dispatchers**: intentionally kept on `create_session_with_metadata` (autonomous, not user-triggered)

### Dispatcher Helper

Extracted `write_execution_trace` helper on `TaskDispatcher` to avoid 4 duplicate blocks:

```rust
async fn write_execution_trace(&self, task_id: &str, trace_id: &str) {
    if let Err(e) = self.db.update_task_execution_trace_id(task_id, trace_id).await {
        warn!(task_id = %task_id, error = %e, "failed to write execution_trace_id");
    }
}
```

### Cross-Agent Safety

`update_task_execution_trace_id` deliberately does NOT scope by `agent_id` — the dispatcher may write `execution_trace_id` for tasks owned by different agents in cross-agent team scenarios. This is consistent with `try_complete_parent_on_sibling_done` and other cross-agent task traversal patterns.

## Prevention Checklist

1. **New async boundaries that run agent loops** must accept `trace_id: Option<String>` and use `unwrap_or_else(generate_trace_id)` — never unconditionally call `generate_trace_id()`.

2. **New task dispatch paths** must generate a trace_id, pass it to the agent params, and call `write_execution_trace` after the run completes.

3. **New session creation at dispatch sites** should use `create_session_with_parent` when the session originates from a user interaction (callback, skill_run). Autonomous sessions (heartbeat, reflection) use `create_session_with_metadata` with no parent.

4. **Schema changes** touching tasks or sessions must update: `TASK_COLUMNS`, `TASK_COLUMN_COUNT`, `Task` struct, `row_to_task()`, clean-slate `create_schema()`, and the `unified_timeline` VIEW constant — all atomically. See `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md` for the column index drift hazard.

5. **"Follow the None" audit**: grep for `trace_id: None` or `created_trace_id: None` where context carries a trace_id — any such site is a bug.

## Key Files

- `crates/mika-agent/src/db.rs` — Schema, Task/Session structs, TASK_COLUMNS, row_to_task, migration v10→v11, update_task_execution_trace_id, create_session_with_parent
- `crates/mika-agent/src/async_db.rs` — Async wrappers for new DB methods
- `crates/mika-agent/src/agent.rs` — TeamAgentParams.trace_id, SilentAgentParams.trace_id, run_team_agent_inner_impl, run_silent_inner
- `crates/mika-agent/src/task_engine/dispatcher.rs` — write_execution_trace helper, all 4 dispatch methods updated
- `crates/mika-agent/src/teams/engine.rs` — trace_id propagation into TeamAgentParams (both execute_tasks and run_single_agent_task)
- `crates/mika-agent/src/tools/delegate_task.rs` — ctx.trace_id propagation
