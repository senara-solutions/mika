---
status: pending
priority: p2
issue_id: 708
tags: [code-review, performance]
dependencies: []
---

# DashMap broadcaster leak on spawned task panic

## Problem Statement
The broadcaster cleanup `broadcasters.remove(&task_id_clone)` is the last line of the spawned closure in `handle_message_stream`. If the agent loop panics, the tokio::spawn task is cancelled and `remove` is never called. The DashMap entry leaks permanently, causing slow memory growth and stale resubscribe lookups.

## Findings
- `crates/mika-agent/src/server/a2a.rs` lines 358-449: cleanup at line 448 is last statement
- Each leaked entry holds a `broadcast::Sender<StreamEvent>` with 32-slot ring buffer

## Proposed Solutions
Use a RAII guard pattern:
```rust
struct BroadcasterGuard { map: Arc<DashMap<...>>, key: String }
impl Drop for BroadcasterGuard { fn drop(&mut self) { self.map.remove(&self.key); } }
```
Create the guard at the start of the spawned closure so cleanup happens on any exit path including panics.

## Acceptance Criteria
- [ ] Broadcaster entries are cleaned up even if the spawned task panics
