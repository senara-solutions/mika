---
status: pending
priority: p3
issue_id: "116"
tags: [code-review, architecture, phase2]
dependencies: []
---

# No Graceful Shutdown for Database Thread

## Problem Statement

The `AsyncDatabase` background thread exits when all `mpsc::Sender` clones are dropped (channel closes). There is no explicit shutdown method to:
1. Wait for in-flight operations to complete
2. Run final cleanup (WAL flush, VACUUM, checkpoint)
3. Confirm the thread has exited

In Phase 1 CLI this is fine (process exits). Phase 2 HTTP server needs graceful shutdown for clean deployments.

## Findings

- **Source:** architecture-strategist, pattern-recognition-specialist (5D)
- **Location:** `crates/mika-agent/src/async_db.rs` line 30 — detached thread, no JoinHandle stored

## Proposed Solutions

### Option 1: Store JoinHandle + add shutdown() method
- **Pros**: Clean shutdown, can run final operations
- **Cons**: Slightly more complex struct
- **Effort**: Small
- **Risk**: Low

### Option 2: Shutdown oneshot channel
- **Pros**: Can signal and wait
- **Cons**: More moving parts
- **Effort**: Small
- **Risk**: Low

## Recommended Action

_To be filled during triage — defer to Phase 2_

## Technical Details

- **Affected Files**: `crates/mika-agent/src/async_db.rs`

## Acceptance Criteria

- [ ] `AsyncDatabase::shutdown()` method exists
- [ ] Waits for in-flight operations to complete
- [ ] Runs final cleanup (configurable)

## Work Log

### 2026-02-24 - Identified in v4 Code Review
**By:** Multi-agent review (architecture-strategist)

## Resources

- Commit under review: 38a843b
