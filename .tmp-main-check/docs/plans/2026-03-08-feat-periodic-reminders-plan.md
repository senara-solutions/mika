---
title: "feat: Add periodic reminder support to create_reminder tool"
type: feat
status: completed
date: 2026-03-08
---

# feat: Add periodic reminder support to create_reminder tool

## Overview

The `create_reminder` tool only creates one-shot reminders (`trigger_type="time"`). Users like Micha expect to say "remind me every Monday at 9am" and get a recurring reminder, but the tool has no `recurrence` parameter. The task engine already fully supports recurring tasks via `trigger_type="recurring"` + `cron_expr`, so the fix is to expose this capability through the user-facing `create_reminder` tool.

## Problem Statement

The `create_reminder` tool hardcodes `trigger_type: "time"` and only accepts `fire_at` + `message`. When a user asks for a periodic reminder, the agent has no way to create one — `create_task` (which supports `recurring`) was removed from `default_tools()`. The system prompt also only mentions one-shot reminders.

## Proposed Solution

Add an optional `recurrence` parameter to `create_reminder`. When provided, the tool creates a `trigger_type="recurring"` task with a computed `cron_expr` instead of a one-shot `trigger_type="time"` task.

### Key Design Decisions

1. **Agent computes the cron expression** — The LLM is responsible for converting natural language ("every Monday at 9am") into a 6-field cron expression ("0 0 9 * * 1"). This avoids building a natural-language-to-cron parser in Rust and leverages what LLMs are good at.
2. **`fire_at` becomes optional** — For recurring reminders, the first fire time is computed from the cron expression. `fire_at` remains required for one-shot reminders (no `recurrence`).
3. **Status `recurring_active`** — Recurring tasks use this status (already supported by `get_user_visible_tasks`, `list_reminders`, and the task engine).

## Technical Considerations

- The `cron` crate (already a dependency) handles 6-field cron parsing and next-fire computation via `next_fire_from_cron()` in `task_engine/cron.rs`
- `get_user_visible_tasks()` already includes `recurring_active` in its WHERE clause — no DB query changes needed
- The task engine's `fire_task()` already re-enqueues recurring tasks after dispatch via `reenqueue_tx` — no engine changes needed
- `cancel_reminder` delegates to `cancel_task` which works on any task status — no changes needed

## Acceptance Criteria

- [x] `create_reminder` accepts an optional `cron_expr` parameter (6-field cron expression)
- [x] When `cron_expr` is provided, the tool creates a `trigger_type="recurring"` task with `status="recurring_active"`
- [x] When `cron_expr` is provided, `fire_at` is not required (first fire computed from cron)
- [x] When `cron_expr` is NOT provided, behavior is unchanged (one-shot, `fire_at` required)
- [x] Invalid cron expressions return a clear error message
- [x] `list_reminders` displays cron expression for recurring reminders
- [x] System prompt mentions that `create_reminder` supports periodic reminders via `cron_expr`
- [x] All existing tests continue to pass
- [x] New tests cover: recurring creation, cron validation, fire_at optional for recurring, list shows recurrence

## MVP

### `crates/mika-agent/src/tools/create_reminder.rs`

Changes:
1. Add optional `cron_expr` property to `input_schema`
2. In `execute()`:
   - If `cron_expr` is provided: validate via `next_fire_from_cron()`, create task with `trigger_type="recurring"`, `status` will be set by DB default to `pending` (engine sets `recurring_active` on first fire — actually, we should set initial status)
   - If `cron_expr` is NOT provided: existing one-shot behavior (require `fire_at`)
3. Update tool description to mention periodic support

### `crates/mika-agent/src/tools/list_reminders.rs`

Changes:
1. Show `cron_expr` for recurring reminders in the output (e.g., `"(recurring: 0 0 9 * * 1)"`)

### `crates/mika-agent/src/prompt.rs`

Changes:
1. Update the reminder guidance line to mention `cron_expr` for periodic reminders

### Test cases

```rust
// crates/mika-agent/src/tools/create_reminder.rs

#[tokio::test]
async fn test_create_recurring_reminder() {
    // cron_expr="0 0 9 * * 1" (every Monday 9am UTC), no fire_at needed
    // Assert: task created with trigger_type="recurring", cron_expr set
}

#[tokio::test]
async fn test_create_recurring_reminder_invalid_cron() {
    // cron_expr="not valid"
    // Assert: error with helpful message
}

#[tokio::test]
async fn test_create_reminder_no_cron_requires_fire_at() {
    // No cron_expr, no fire_at → error
    // Existing behavior preserved
}

// crates/mika-agent/src/tools/list_reminders.rs

#[tokio::test]
async fn test_list_reminders_shows_recurrence() {
    // Create a recurring reminder
    // Assert: list output includes cron expression
}
```

## Sources

- Existing recurring support: `crates/mika-agent/src/task_engine/engine.rs` (re-enqueue logic at line 388)
- Cron helper: `crates/mika-agent/src/task_engine/cron.rs:10` (`next_fire_from_cron`)
- Current tool: `crates/mika-agent/src/tools/create_reminder.rs`
- Task types: `crates/mika-agent/src/task_engine/types.rs` (trigger_type, task_status constants)
- DB query: `crates/mika-agent/src/db.rs:997` (`get_user_visible_tasks` already includes `recurring_active`)
- System prompt: `crates/mika-agent/src/prompt.rs:310`
- Past solution: `docs/solutions/runtime-errors/reminders-never-fire-at-scheduled-time.md`
