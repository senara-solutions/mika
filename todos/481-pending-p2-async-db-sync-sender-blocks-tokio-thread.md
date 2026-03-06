---
status: pending
priority: p2
issue_id: "481"
tags: [code-review, performance, database]
dependencies: []
---

# AsyncDatabase SyncSender::send Blocks Tokio Worker Thread Under Backpressure

## Problem Statement

`AsyncDatabase::with_db` creates a `std::sync::mpsc::sync_channel(512)` and calls
`SyncSender::send()` while holding a `std::sync::Mutex`. When the channel is full (DB thread
falls 512 operations behind), `SyncSender::send()` **blocks the calling thread** — a synchronous
blocking call made from an `async fn`. If all tokio worker threads are blocked waiting on a full
channel, the async scheduler stalls. In single-customer containers with 1–2 CPU cores, this
means a slow VACUUM or FTS rebuild in the DB thread can cause the entire server to stop
processing requests.

## Findings

- **Source**: architecture-strategist and performance-oracle reviews
- **Location**: `crates/mika-agent/src/async_db.rs:99–117`
- `std::sync::Mutex` is held across `SyncSender::send()` call
- If multiple tasks simultaneously try to queue DB operations on a full channel, only one can
  proceed at a time (mutex) and that one will block the tokio thread
- Normal load (handful of tasks + one agent turn): channel never fills, non-issue
- Failure mode: slow DB operation + concurrent DB-heavy code path (e.g., FTS reindex)

## Proposed Solutions

### Option A: Clone sender before releasing mutex, call send without holding lock (Recommended)
```rust
let sender = {
    let guard = self.inner.sender.lock().expect("sender lock poisoned");
    guard.as_ref().ok_or_else(|| anyhow!("database has been shut down"))?.clone()
};
// mutex released here — send does not block the mutex for other callers
sender.send(Box::new(move |db| { let _ = tx.send(f(db)); }))
      .map_err(|_| anyhow!("database thread has stopped"))?;
```
- **Pros**: Mutex not held during potentially-blocking send, easy change
- **Cons**: send can still block the tokio thread if channel is full
- **Effort**: Small | **Risk**: Low

### Option B: Use tokio::sync::mpsc channel + spawn_blocking for send
Replace `std::sync::mpsc::sync_channel` with `tokio::sync::mpsc::channel` and send via
`spawn_blocking` when needed.
- **Pros**: Fully non-blocking from async context
- **Cons**: Larger refactor, requires restructuring the DB thread receiver
- **Effort**: Medium | **Risk**: Medium

## Acceptance Criteria

- [ ] `with_db` does not hold `std::sync::Mutex` across a blocking call
- [ ] Behavior under DB backpressure is documented with a comment
- [ ] All existing tests pass

## Work Log

- 2026-03-06: Identified by architecture-strategist and performance-oracle reviews of feat/unified-task-engine
