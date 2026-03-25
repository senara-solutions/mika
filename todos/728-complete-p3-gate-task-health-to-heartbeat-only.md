---
status: pending
priority: p3
issue_id: 728
tags: [code-review, architecture]
---

# Gate task health injection to heartbeat trigger only

## Problem Statement

`get_task_health_summary()` and `list_preferences()` are called for ALL silent triggers (Heartbeat, Reflection, Callback, SkillRun). The `<task-health-instructions>` directive ("Review the task health summary... take action...") is injected regardless of trigger type. For Callback/Reflection/SkillRun triggers, the agent has a different job and the health check instructions create confusion and waste tokens.

## Findings

- Previous `pending_work_items` was also loaded unconditionally — this follows the existing pattern
- New instructions are more directive ("take it autonomously") vs old passive ("consider notifying")
- Data loading cost is negligible, but prompt token waste is not
- Reflection trigger has its own detailed instruction set about memory housekeeping

## Proposed Solutions

Gate the data loading on heartbeat trigger only:
```rust
let (task_health, stored_preferences) = if matches!(&trigger, SilentTrigger::Heartbeat) {
    (db.get_task_health_summary().await.ok(), db.list_preferences().await.unwrap_or_default())
} else {
    (None, vec![])
};
```

## Acceptance Criteria

- [ ] Task health and preferences only loaded/injected for Heartbeat triggers
- [ ] Reflection, Callback, and SkillRun triggers skip health data
