---
status: pending
priority: p3
issue_id: "516"
tags: [code-review, simplicity, refactor]
dependencies: []
---

# `SilentTrigger::Callback` Carries `task_id` Only for Prompt String — Unnecessary Field

## Problem Statement

`SilentTrigger::Callback` has three fields: `task_id`, `label`, `result`. `task_id` is used in only one place: string formatting into the system prompt. It is not used to re-query the DB, stored, or logged. Passing it through `SilentAgentParams → SilentTrigger → match arm` is pure pass-through with no additional value.

## Findings

- **Source**: simplicity-reviewer (F-5)
- **Location**: `crates/mika-agent/src/agent.rs:1008-1017`, `crates/mika-agent/src/task_engine/dispatcher.rs`

```rust
SilentTrigger::Callback { task_id, label, result } => {
    format!("... Task: '{label}' (ID: {task_id})\n\nResult:\n{result}\n...")
}
```

`task_id` is available in `dispatch_resume_agent` via `task.id`. It can be included in the `result` context string before constructing the trigger, eliminating the extra enum field.

## Proposed Solutions

### Option A: Drop `task_id`, include in result context string in dispatcher (Recommended)

In `dispatch_resume_agent`:
```rust
let context = format!("Task '{}' (ID: {}) completed with result:\n{}",
                       task.label, task.id, result);
let trigger = SilentTrigger::Callback { label: task.label.clone(), result: context };
```

In `run_silent_inner`:
```rust
SilentTrigger::Callback { label, result } => {
    format!("A background task '{label}' has completed.\n\n{result}\n\nNotify the user via send_message.")
}
```

Impact: removes one field from enum, one field assignment in dispatcher, one destructuring in two match arms.

- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] `SilentTrigger::Callback` has `label` and `result` fields only (no `task_id`)
- [ ] `dispatch_resume_agent` includes task ID in the result context string
- [ ] All match arms updated
- [ ] Existing callback dispatch tests pass

## Work Log

- 2026-03-06: Identified by simplicity-reviewer of feat/unified-task-engine
