---
status: complete
priority: p1
issue_id: "027"
tags: [code-review, architecture, performance, rust-v2]
dependencies: []
---

# Synchronous SQLite Calls Block Tokio Async Runtime

## Problem Statement

All `Database` methods use synchronous `rusqlite::Connection` I/O, called directly from `async fn run_agent()`. This blocks the Tokio worker thread during every database operation. With the agent loop performing 3+ DB calls per turn (save_message, get_all_core_memory, load_recent_messages) plus 1-3 per tool call, the executor is blocked repeatedly.

**Why it matters:** When the axum HTTP server is added (Phase 2), blocking calls will starve other async tasks (health checks, typing indicators, heartbeat timers). Must be fixed before Phase 2.

## Findings

- **Source:** Performance Oracle (CRITICAL-1), Architecture Strategist (F, R1)
- **Location:** `crates/mika-agent/src/agent.rs` calling `crates/mika-agent/src/db.rs`
- **Evidence:** `db.save_message(...)`, `db.load_recent_messages(...)`, `db.get_all_core_memory(...)` all synchronous, called from async context

## Proposed Solutions

### Option A: Dedicated thread with mpsc channel (Recommended)
- Move `Database` to a `std::thread::spawn` with `tokio::sync::mpsc` for commands and `oneshot` for replies
- Expose `AsyncDatabase` wrapper with async methods
- **Pros:** Clean separation, Connection stays on one thread (no Send issues), natural for the single-consumer model
- **Cons:** More boilerplate (one enum variant per DB method)
- **Effort:** Medium
- **Risk:** Low

### Option B: Use `tokio_rusqlite` crate
- Drop-in async wrapper for rusqlite
- **Pros:** Minimal code changes, well-tested crate
- **Cons:** New dependency, less control
- **Effort:** Small
- **Risk:** Low

### Option C: spawn_blocking per call
- Wrap each DB call in `tokio::task::spawn_blocking`
- **Pros:** Simplest change
- **Cons:** Connection is !Send so this requires restructuring
- **Effort:** Medium
- **Risk:** Medium (borrow checker complexity)

## Acceptance Criteria

- [ ] No synchronous I/O on Tokio worker threads
- [ ] Agent loop and tools use async database methods
- [ ] Health check endpoint can respond while agent loop is running
- [ ] All existing tests still pass
