---
status: pending
priority: p2
issue_id: "467"
tags: [code-review, architecture, performance, database]
dependencies: []
---

# 467 · `AsyncDatabase` channel is unbounded — no backpressure under load

## Problem Statement

`std::sync::mpsc::channel()` creates an unbounded channel. Under heavy
tick-loop throughput (many tasks firing per second), or during a slow DB
operation (VACUUM, large FTS query, vec0 similarity search), the DB thread
falls behind and the channel queue grows without limit. This can cause
unbounded memory growth in long-running deployments, particularly in server
mode where the tick loop fires every second.

## Findings

- **Location:** `crates/mika-agent/src/async_db.rs:43`
- Pattern previously flagged: todo #111 (VACUUM blocking DB thread)
- The tick loop fires up to `MAX_PER_TICK = 10` tasks per second, each potentially making 3 DB round-trips (get_task, update_task_status, set_task_fired = 30 ops/sec under load)

## Proposed Solutions

### Option A — Bounded `sync_channel` (recommended)
```rust
let (tx, rx) = std::sync::mpsc::sync_channel(512);
```
Callers block when the channel is full, providing natural backpressure. The tick loop slows down rather than accumulating work unboundedly.

**Pros:** Prevents memory unbounded growth. Simple change.
**Cons:** Blocking callers need to run in `spawn_blocking` contexts — already the case via `with_db`.
**Effort:** Small | **Risk:** Low

### Option B — Add monitoring / alerting
Keep unbounded but add a channel-length metric emitted via tracing.
**Cons:** Does not prevent the problem.
**Effort:** Small | **Risk:** N/A (mitigation only)

## Recommended Action

Option A with a bound of 512 and a `CLAUDE.md` note on the rationale.

## Technical Details

- **Affected files:** `crates/mika-agent/src/async_db.rs`

## Acceptance Criteria

- [ ] `channel()` replaced with `sync_channel(512)` (or chosen bound)
- [ ] Bound documented in code comment
- [ ] Verify no deadlock: `with_db` is called from `spawn_blocking` or async context without holding other locks

## Work Log

- 2026-03-06: Identified by architecture review agent (ARCH-5)
