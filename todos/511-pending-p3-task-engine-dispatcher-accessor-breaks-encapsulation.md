---
status: pending
priority: p3
issue_id: "511"
tags: [code-review, architecture, encapsulation]
dependencies: []
---

# `TaskEngine::dispatcher()` Exposes Inner Dispatcher — Future Misuse Risk

## Problem Statement

`TaskEngine::dispatcher()` returns a clone of the inner `Arc<TaskDispatcher>`, providing direct access to dispatch functions outside the engine's dedup mechanism. A future caller using this accessor to dispatch a `time` or `recurring` task would bypass `claim_and_fire_task` and the dedup `HashSet`, potentially double-firing.

## Findings

- **Source**: architecture-strategist (F-7 Low)
- **Location**: `crates/mika-agent/src/task_engine/engine.rs:62-66`

```rust
pub fn dispatcher(&self) -> Arc<TaskDispatcher> {
    self.dispatcher.clone()
}
```

Currently only used in `handle_task_complete` to get the dispatcher for `dispatch_completed_callback`. This is safe because callback tasks never pass through `claim_and_fire_task`. But the accessor is public and undocumented — a future caller could use it to directly dispatch a time/recurring task, bypassing the engine's dedup logic and causing double-fires.

Additionally, the performance-oracle recommends storing `Arc<TaskDispatcher>` directly on `AgentState` to eliminate the engine mutex from the `handle_task_complete` hot path entirely.

## Proposed Solutions

### Option A: Store dispatcher on `AgentState` directly (Recommended)

```rust
// In AgentState:
pub dispatcher: Arc<TaskDispatcher>,

// In handle_task_complete:
let dispatcher = agent_state.dispatcher.clone();
tokio::spawn(async move {
    dispatcher.dispatch_completed_callback(&completed_task).await
});
```

Remove `TaskEngine::dispatcher()`. No mutex acquisition needed for callback dispatch hot path.

- **Effort**: Small | **Risk**: Low

### Option B: Narrow the API to a type-checked method

```rust
pub fn fire_callback_task(&self, task: &Task) -> JoinHandle<()> {
    assert_eq!(task.trigger_type, trigger_type::CALLBACK,
               "fire_callback_task requires trigger_type=callback");
    let dispatcher = self.dispatcher.clone();
    let task = task.clone();
    tokio::spawn(async move {
        dispatcher.dispatch_completed_callback(&task).await.ok();
    })
}
```

Keeps the invariant enforced at the engine boundary. Callers can't accidentally dispatch non-callback tasks.

- **Effort**: Small | **Risk**: Low

## Acceptance Criteria

- [ ] `TaskEngine::dispatcher()` is either removed or replaced with a narrower API
- [ ] `handle_task_complete` does not hold the engine mutex during agent dispatch
- [ ] Existing callback dispatch tests pass

## Work Log

- 2026-03-06: Identified by architecture-strategist and performance-oracle reviews of feat/unified-task-engine
