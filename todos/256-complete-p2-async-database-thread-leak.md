---
status: complete
priority: p2
issue_id: 256
tags: [code-review, performance, quality]
dependencies: []
---

# AsyncDatabase Thread Leak in TeamEngine

## Problem Statement

TeamEngine opens an AsyncDatabase per agent (spawns an OS thread each). When the engine is dropped, the threads are not explicitly shut down -- they rely on the mpsc channel closing when the sender side is dropped. JoinHandles are never joined, so threads may still be running when `execute()` returns. This leaks OS threads and can leave SQLite connections open longer than necessary.

## Findings

- **File:** `crates/mika-agent/src/teams/engine.rs` lines 121-148
- Each agent in the team gets its own `AsyncDatabase` instance, which spawns a dedicated OS thread
- The `AsyncDatabase` uses an mpsc channel for communication; when the sender is dropped, the thread should eventually exit
- However, `JoinHandle`s are never explicitly joined, so there is no guarantee the thread has finished when `execute()` returns
- In the worst case, threads may still be processing a final database operation after the engine is dropped
- With a 5-agent team, this means 5 OS threads that may linger after execution

## Proposed Solutions

Call `shutdown()` on all `AsyncDatabase` instances before returning from `execute()`. Add shutdown calls in a cleanup block at the end of `execute()` to ensure deterministic thread termination.

```rust
// At the end of execute():
for agent_db in &self.agent_databases {
    agent_db.shutdown().await;
}
```

If `AsyncDatabase` does not have a `shutdown()` method, consider:
1. Adding one that drops the sender and joins the thread handle
2. Implementing `Drop` on `TeamEngine` that signals shutdown (though async cleanup in Drop is tricky)
3. Using a dedicated cleanup method that must be called explicitly

## Technical Details

- `AsyncDatabase` wraps sync `Database` with a dedicated OS thread + `mpsc` channel (closure-based dispatch)
- The OS thread runs a loop receiving closures over the channel; it exits when the channel closes
- Channel closing happens when all senders are dropped, but this is non-deterministic with respect to `execute()` returning
- SQLite connections held by the thread may block other operations if not cleanly closed
- Consider whether `AsyncDatabase` already has a `shutdown()` or `close()` method from its existing implementation

## Acceptance Criteria

- [ ] All AsyncDatabase threads are cleanly shut down when TeamEngine completes execution
- [ ] JoinHandles are joined to ensure threads have fully exited
- [ ] SQLite connections are properly closed before `execute()` returns
- [ ] No OS thread leaks observable under repeated team executions
- [ ] All existing tests pass

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from PR #13 code review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
- AsyncDatabase implementation: `crates/mika-common/` (or `crates/mika-agent/`)
