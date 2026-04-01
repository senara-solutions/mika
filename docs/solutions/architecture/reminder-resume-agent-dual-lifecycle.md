---
title: "Reminder resume_agent: dual lifecycle in dispatch_resume_agent"
category: architecture
date: 2026-04-01
tags: [task-engine, dispatcher, silent-trigger, reminder, resume-agent, lifecycle]
issue: 363
---

# Reminder resume_agent: dual lifecycle in dispatch_resume_agent

## Problem

`create_reminder` hardcoded `action_type: "send_message"`, which only sends a Telegram notification. In CLI mode (no `MessageSender`), reminders were silently dropped. CI follow-up reminders never woke the agent to check CI and merge.

Adding `resume_agent` as an action type for reminders required reusing `dispatch_resume_agent`, but that method was designed for the **callback** lifecycle — a fundamentally different task state flow.

## Root Cause

The `dispatch_resume_agent` method assumed the callback lifecycle:
- **Context source:** `task.result` (set by external process completing the task)
- **Status at dispatch:** `completed` or `failed`
- **Post-dispatch:** calls `mark_task_delivered()`
- **Session prefix:** `callback-{uuid}`
- **Trust level:** untrusted (external data wrapped in `<callback_result trust="untrusted">`)

Reminder tasks follow a different lifecycle:
- **Context source:** `action_config.text` (set at creation time)
- **Status at dispatch:** `in_progress` (claimed by `fire_task`)
- **Post-dispatch:** `fire_task()` handles completion — do NOT call `mark_task_delivered`
- **Session prefix:** `reminder-{uuid}`
- **Trust level:** trusted (user-authored message, no untrusted wrapping)

## Solution

Refactored `dispatch_resume_agent` to branch on `task.trigger_type`:

```rust
let is_callback = task.trigger_type == "callback";

let (trigger, session_prefix, session_trigger_meta) = if is_callback {
    // Callback path: read from task.result, use SilentTrigger::Callback
    // ...
} else {
    // Reminder path: read from action_config.text, use SilentTrigger::Reminder
    let config: serde_json::Value = serde_json::from_str(&task.action_config)...;
    let message = config["text"].as_str().unwrap_or(&task.label).to_string();
    (SilentTrigger::Reminder { task_id, message }, "reminder", ...)
};

// After silent agent run:
if is_callback {
    // Only callbacks mark delivered — reminder lifecycle managed by fire_task()
    self.db.mark_task_delivered(&task.id).await;
}
```

Key design decisions:
1. **New `SilentTrigger::Reminder` variant** — gives proper prompt framing ("A reminder you set has fired") without untrusted-framing tags
2. **Task health injection** — `Reminder` is included in the `matches!()` guard alongside `Heartbeat` and `Callback` for work item awareness
3. **Schema v18 migration** — widened `idx_tasks_unique_reminder` to cover both `send_message` and `resume_agent` action types (excluding `callback` trigger type to avoid blocking callback task creation)

## Prevention

When adding new entry paths to an existing dispatch method:

1. **Map the lifecycle differences first** — status at dispatch, context source, post-dispatch transitions, trust level
2. **Branch on trigger_type, not action_type** — `action_type` tells you *what* to do, `trigger_type` tells you *how* you got here
3. **Guard lifecycle-specific calls** — `mark_task_delivered` is callback-specific; wrapping it behind `if is_callback` prevents state corruption for other paths
4. **Unique index scope** — partial unique indexes must exclude system task types (callbacks) that legitimately create multiple tasks with the same label
5. **Session prefix matters** — distinct prefixes (`callback-`, `reminder-`) enable independent pruning, dashboard filtering, and observability
