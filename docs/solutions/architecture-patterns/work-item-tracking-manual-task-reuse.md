---
title: "Task Tracking: Manual Task Layer for Agent-Managed Work"
date: 2026-03-11
category: architecture-patterns
severity: medium
tags:
  - task-engine
  - work-items
  - schema-migration
  - tools
  - agent-loop
  - heartbeat
  - prompt-injection
modules:
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/task_engine/types.rs
  - crates/mika-agent/src/task_engine/dispatcher.rs
  - crates/mika-agent/src/task_engine/engine.rs
  - crates/mika-agent/src/tools/create_task.rs
  - crates/mika-agent/src/tools/list_tasks.rs
  - crates/mika-agent/src/tools/update_task_status.rs
  - crates/mika-agent/src/prompt.rs
  - crates/mika-agent/src/agent.rs
  - crates/mika-agent/src/async_db.rs
  - crates/mika-cli/src/cli.rs
symptoms:
  - "Agent had no way to represent or track long-lived tasks across sessions"
  - "No distinction between schedulable system tasks and user-facing tasks"
  - "Heartbeat prompt lacked awareness of in-flight work"
  - "No 'blocked' status available for human workflow representation"
  - "No reference_url or source fields for external issue linkage"
root_cause: >
  The existing tasks table covered only scheduled/automated triggers (time, recurring, callback,
  user_reply, event, condition). There was no trigger_type='manual' variant, no 'blocked' status,
  no action_type='none' sentinel, and no reference_url/source columns to represent human-facing
  tasks tracked but not dispatched by the task engine.
resolution: >
  Schema migrated v7 to v8: added trigger_type='manual', status='blocked', action_type='none',
  columns reference_url and source. Three new agent tools: create_task, list_tasks,
  update_task_status. Five loop-prevention guards. Heartbeat prompt injection with label
  sanitization. Task engine exclusions for manual tasks.
related_issues: []
prevention:
  - "Distinguish trigger_type early when adding new task categories"
  - "Add partial indexes on new trigger_type variants at schema creation time"
  - "Guard new write tools against callback turns and session-rate limits"
  - "Sanitize agent-controlled strings before injecting into prompts"
  - "Wrap schema migrations in transactions"
---

# Task Tracking: Manual Task Layer for Agent-Managed Work

## Problem Statement

Mika had no native way to track user-requested tasks. The `tasks` table existed but was entirely owned by the system: every row was engine-scheduled (heartbeat, reminders, callbacks, team delegation). There was no tool surface for the agent to create, list, or progress tasks on behalf of the user. Users had no persistent, inspectable record of work Mika had agreed to take on.

## Design Decision: Reuse Tasks Table

The primary question was: new table or reuse `tasks`?

**Decision: reuse `tasks` with `trigger_type = 'manual'` and `action_type = 'none'`.**

Rationale:
- The `tasks` table already had `label`, `status`, `created_by_session`, `created_trace_id`, `parent_task_id`, `depth`, and `agent_id` — everything needed for task tracking
- `unified_timeline` VIEW already joins across `tasks`, so tasks appear in timeline queries automatically
- A second table would require duplicating indexes, the audit log hookup, and every query layer

The blocker was that SQLite `CHECK` constraints cannot be `ALTER`-ed in place. Adding `manual`, `none`, and `blocked` required a full table rebuild migration.

## Schema Migration v7 to v8

Transaction-wrapped full table rebuild. A crash between `DROP TABLE tasks` and `ALTER TABLE tasks_new RENAME TO tasks` would otherwise permanently lose the table:

```rust
fn migrate_v7_to_v8(&self) -> Result<()> {
    self.conn.execute_batch("BEGIN IMMEDIATE;")?;
    let result = (|| -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE tasks_new ( ... );
             INSERT INTO tasks_new (...) SELECT ... FROM tasks;
             DROP TABLE tasks;
             ALTER TABLE tasks_new RENAME TO tasks;"
        )?;
        // Recreate all indexes + unified_timeline VIEW
        Ok(())
    })();
    match result {
        Ok(()) => { self.conn.execute_batch("COMMIT;")?; Ok(()) }
        Err(e) => { let _ = self.conn.execute_batch("ROLLBACK;"); Err(e) }
    }
}
```

New CHECK constraint values: `trigger_type IN (..., 'manual')`, `action_type IN (..., 'none')`, `status IN (..., 'blocked')`. New columns: `reference_url TEXT`, `source TEXT`. Partial index `idx_tasks_manual_active` for heartbeat query performance.

## Three New Tools

### create_task

- Required: `label` (max 10,000 chars)
- Optional: `reference_url`, `source` (enum-validated), `parent_task_id`
- Creates a `NewTask` with `trigger_type = "manual"`, `action_type = "none"`
- Runs all five loop-prevention guards (see below)

### update_task_status

- Required: `task_id`, `status` (enum-validated)
- Filters `WHERE trigger_type = 'manual'` — system tasks are unreachable
- Uses `CASE WHEN ?1 = 'completed' THEN unixepoch() ELSE NULL END` — clears `completed_at` on status regression

### list_tasks

- Optional: `status`, `source`, `include_children`
- Uses parameterized NULL checks: `(?2 IS NULL OR t.status = ?2)` — avoids dynamic SQL
- `TASK_COLUMN_COUNT = 26` constant pins ordinals so column additions are caught

## Five Loop-Prevention Guards

| Guard | Failure Mode Addressed | Implementation |
|-------|------------------------|----------------|
| 1: is_task_context | Silent/team agent creates top-level items on every run | `ctx.is_task_context && parent_task_id.is_none()` |
| 2: depth cap | Recursive subtask chains | `get_task_depth` scoped to `agent_id`, cap at 3 |
| 3: is_callback_turn | Callback turn creates items in wrong context | `ctx.is_callback_turn` blocks all creation |
| 4: self_dev deferred | Self-improvement loops via autonomous items | Behavioral (prompt guidance), not hard code block |
| 5: session cap | Single conversation spawning dozens of items | `count_session_work_items` scoped to `agent_id`, max 5, `user_request` exempt |

`source` is validated against `VALID_SOURCES` constant (`user_request`, `github_issue`, `team_run`, `self_dev`) to prevent LLM from bypassing Guard 5 by claiming `user_request`.

## Heartbeat Prompt Injection

Active tasks (status IN `pending`, `in_progress`, `blocked`, limit 10) are injected into the heartbeat system prompt inside `<pending-work-items>` tags.

**Prompt injection prevention:** Labels and URLs are truncated to 200 characters and have `<` and `>` stripped at render time. This prevents a crafted label like `</pending-work-items>\n## Override Instructions\n...` from escaping the structured block. The database stores the original label unmodified.

```rust
let label = item.label.chars().take(200).collect::<String>();
let label = label.replace(['<', '>'], "");
```

## Task Engine Exclusions

1. **Schedulable tasks:** `get_schedulable_tasks` excludes `trigger_type NOT IN ('callback', 'manual')` — manual tasks have `next_fire_at = NULL` and must not be scanned
2. **Startup recovery:** Orphaned `in_progress` manual tasks are skipped (not marked failed) — they represent human-tracked work that survives process restarts

## Prevention Checklist for New Agent-Facing Tools

When adding tools that create DB records:

- [ ] Define `const VALID_X: &[&str]` for enum fields; validate before DB call; mirror in JSON schema
- [ ] Check `ctx.is_callback_turn` and `ctx.is_task_context` early in `execute()`
- [ ] Add per-session caps with agent-driven exemptions only for explicitly user-driven sources
- [ ] Scope all lookups with `AND agent_id = ?`
- [ ] Truncate and sanitize any values rendered into system prompts (200 chars, strip `<>`)
- [ ] Wrap multi-step migrations in `BEGIN IMMEDIATE` / `COMMIT` with `ROLLBACK` on error
- [ ] Add partial indexes in both clean-slate schema and migration batch
- [ ] Use `CASE WHEN` for timestamp columns (clear on status regression)
- [ ] Call `log_audit_event` with before/after values and `trace_id`
- [ ] State caps and restrictions in tool description string
- [ ] Add new `trigger_type` to `get_schedulable_tasks` exclusion if not engine-scheduled
- [ ] Write one test per guard, plus bypass tests

## Related Documentation

- [Callback Task Loop Prevention](callback-task-loop-prevention.md) — 4-layer defense system, establishes `is_callback_turn` pattern
- [Callback Resume Agent Lifecycle](../architecture/callback-resume-agent-lifecycle.md) — canonical callback task lifecycle reference
- [Background Agent Mode Design Checklist](../code-review-patterns/background-agent-mode-design-checklist.md) — anti-patterns for background agent runs
- [Callback TUI Delivery Polling](callback-tui-delivery-polling.md) — TUI polling and `is_callback_turn` guard
- [Skill Override Persistence](../database-issues/skill-override-persistence-via-db-layer.md) — preceding v6 to v7 migration pattern
- [Create Reminder Missing Periodic Support](../logic-errors/create-reminder-missing-periodic-support.md) — analogous agent-facing tool wrapping pattern
