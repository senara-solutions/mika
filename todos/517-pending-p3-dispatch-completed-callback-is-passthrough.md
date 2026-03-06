---
status: pending
priority: p3
issue_id: "517"
tags: [code-review, simplicity, refactor]
dependencies: []
---

# `dispatch_completed_callback` Is a One-Liner Wrapper — Unnecessary Indirection

## Problem Statement

`dispatch_completed_callback` is a public method that does nothing except call `dispatch_resume_agent`. It provides no additional validation, logging, or error handling beyond its private delegate. The only caller is `handlers.rs:401`.

## Findings

- **Source**: simplicity-reviewer (F-3)
- **Location**: `crates/mika-agent/src/task_engine/dispatcher.rs:237-239`

```rust
pub async fn dispatch_completed_callback(&self, task: &Task) -> Result<()> {
    self.dispatch_resume_agent(task).await
}
```

The simplicity-reviewer notes: "The doc comment claims it avoids re-loading the task status, but `dispatch_resume_agent` reads `task.result` from the struct anyway — it doesn't re-query the DB."

Two cleaner options: make `dispatch_resume_agent` `pub(crate)` and call it directly, or restructure the handler to use `dispatcher.dispatch(task_id)` (which re-fetches from DB — one extra read on what is already an async path).

## Proposed Solutions

### Option A: Make `dispatch_resume_agent` pub(crate), remove the wrapper (Recommended)

```rust
// In dispatcher.rs:
pub(crate) async fn dispatch_resume_agent(&self, task: &Task) -> Result<()> { ... }
// Remove dispatch_completed_callback entirely
```

In `handlers.rs`:
```rust
dispatcher.dispatch_resume_agent(&completed_task).await
```

- **Effort**: Tiny | **Risk**: None

### Option B: Restructure handler to call `dispatcher.dispatch(task_id)`

After `update_task_completed` writes the result to DB, call `dispatch(task_id)` which re-fetches the task and routes by action_type. Eliminates the struct-patch hack (finding 4 in simplicity review) but adds one extra DB read.

- **Effort**: Small | **Risk**: Low

## Acceptance Criteria

- [ ] `dispatch_completed_callback` is removed
- [ ] Handler calls `dispatch_resume_agent` (pub(crate)) or `dispatch(task_id)` directly
- [ ] Existing callback dispatch tests pass

## Work Log

- 2026-03-06: Identified by simplicity-reviewer of feat/unified-task-engine
