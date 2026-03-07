---
status: complete
priority: p2
issue_id: "482"
tags: [code-review, architecture, server]
dependencies: []
---

# TaskEngine Tick Loop JoinHandle Dropped at Server Shutdown (No Clean Shutdown)

## Problem Statement

In `server/mod.rs`, `TaskEngine::spawn_tick_loop(engine)` is called but the returned
`JoinHandle<()>` is not stored anywhere — it is immediately dropped. When `axum::serve`
completes (SIGTERM/Ctrl-C), the graceful shutdown path does not abort or join the tick loops.
Tokio will eventually cancel them during runtime shutdown, but this races with in-flight
dispatched tasks (heartbeat/reflection running a silent agent loop via `tokio::spawn`). These
in-flight tasks may be cut off mid-LLM-call, leaving tasks stuck in `in_progress` status until
the next `startup_recovery()`.

Compare: the CLI correctly stores `poller_handle` in `chat.rs` and calls `.abort()` on agent
switch and exit.

## Findings

- **Source**: architecture-strategist review
- **Location**: `crates/mika-agent/src/server/mod.rs` (line where spawn_tick_loop is called)
- The CLI pattern at `crates/mika-cli/src/commands/chat.rs:449` correctly aborts the handle
- `startup_recovery()` will fix stuck tasks on next restart, but clean shutdown is preferable

## Proposed Solutions

### Option A: Store JoinHandles and abort on shutdown (Recommended)
```rust
let mut tick_handles = Vec::new();
// ... for each agent:
let handle = TaskEngine::spawn_tick_loop(Arc::clone(&task_engine));
tick_handles.push(handle);
// On shutdown signal:
for handle in tick_handles { handle.abort(); }
```
Use `axum::serve(...).with_graceful_shutdown(...)` and abort handles in the shutdown future.
- **Pros**: Clean shutdown, consistent with CLI behavior
- **Effort**: Small | **Risk**: Low

### Option B: Use CancellationToken from tokio-util
Pass a `CancellationToken` into the tick loop and cancel on shutdown.
- **Pros**: Cooperative cancellation, allows in-flight tasks to complete
- **Cons**: More invasive change, adds dependency
- **Effort**: Medium | **Risk**: Low

## Acceptance Criteria

- [ ] Tick loop JoinHandle is stored and aborted/joined during server shutdown
- [ ] Behavior is consistent with CLI's poller_handle.abort() pattern
- [ ] Tasks in_progress at shutdown are recoverable by startup_recovery() on restart

## Work Log

- 2026-03-06: Identified by architecture-strategist review of feat/unified-task-engine
