---
title: create_reminder tool lacks periodic/recurring reminder support
component: crates/mika-agent/src/tools/create_reminder.rs
severity: medium
date_resolved: 2026-03-08
tags: [reminders, cron, recurring, task-engine, tool-parameters]
---

# create_reminder tool lacks periodic/recurring reminder support

## Symptom

Users asking Mika to set periodic reminders (e.g., "remind me every Monday at 9am") received errors or one-shot reminders instead. The agent had no way to create recurring reminders through the user-facing `create_reminder` tool.

## Investigation

1. Checked `create_reminder` tool — hardcodes `trigger_type: "time"` and only accepts `fire_at` + `message`
2. Checked task engine — fully supports `trigger_type: "recurring"` with `cron_expr` (6-field cron)
3. Checked `create_task` tool — supports recurring but was removed from `default_tools()` (test-only)
4. The system prompt only mentioned one-shot reminders with ISO 8601

## Root Cause

Feature gap. The `create_reminder` tool was built for one-shot reminders only. When the unified task engine was implemented (supporting recurring tasks for heartbeat and reflection), the `create_reminder` tool was not extended to expose this capability. The `create_task` tool (which did support recurring) was later removed from `default_tools()`, leaving no user-facing path to periodic reminders.

## Solution

Added an optional `cron_expr` parameter to `create_reminder`:

```rust
// When cron_expr is provided → periodic reminder
let (trigger_type, cron_expr, next_fire_at, display) = if !cron_expr_input.is_empty() {
    // Validate cron, compute next fire time
    let next_fire = next_fire_from_cron(cron_expr_input, now)?;

    // Reject expressions firing more than once per minute (DoS prevention)
    if next_fire - now < 60 {
        if let Ok(second_fire) = next_fire_from_cron(cron_expr_input, next_fire) {
            if second_fire - next_fire < 60 {
                return error("too frequently");
            }
        }
    }

    ("recurring", Some(cron_expr_input), next_fire, format!("periodic ({cron_expr_input})"))
} else {
    // One-shot: existing behavior unchanged
    ("time", None, timestamp, display_time)
};

// Single NewTask construction for both paths
let task = NewTask { trigger_type, cron_expr, next_fire_at, ..shared_fields };
```

Key changes:
- `create_reminder.rs`: Added `cron_expr` parameter, minimum 1-minute interval guard, unified `NewTask` construction
- `list_reminders.rs`: Shows `(recurring: <cron>)` for periodic reminders
- `prompt.rs`: System prompt mentions both one-shot and periodic modes

## Prevention

- When adding new task engine capabilities (trigger types, action types), audit user-facing tools to check if they should expose the new functionality
- The `create_reminder` / `create_task` layering is intentional (user-facing vs low-level), but new task engine features should propagate to the appropriate layer

## Related

- [reminders-never-fire-at-scheduled-time.md](../runtime-errors/reminders-never-fire-at-scheduled-time.md) — original reminder firing bug
- [sqlite-datetime-format-mismatch.md](../database-issues/sqlite-datetime-format-mismatch.md) — INTEGER timestamp storage
- [Unified task engine plan](../../plans/2026-03-04-feat-unified-task-engine-plan.md) — task engine architecture

## Affected Files

| File | Change |
|------|--------|
| `crates/mika-agent/src/tools/create_reminder.rs` | Added `cron_expr` parameter, min interval guard, unified NewTask |
| `crates/mika-agent/src/tools/list_reminders.rs` | Show cron expression for recurring reminders |
| `crates/mika-agent/src/prompt.rs` | Updated system prompt guidance |
| `docs/architecture.md` | Updated tool descriptions |
