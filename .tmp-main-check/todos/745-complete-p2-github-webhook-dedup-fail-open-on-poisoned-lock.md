---
status: pending
priority: p2
issue_id: 745
tags: [code-review, gateway, reliability]
dependencies: []
---

# GitHub webhook LRU cache fail-open on poisoned Mutex

## Problem Statement

The `github_delivery_cache` uses `std::sync::Mutex` which can be poisoned if a thread panics while holding the lock. The current code silently proceeds (fail-open) when the lock is poisoned, which means duplicate deliveries will be processed. While this is the correct availability trade-off, the poisoned state is never logged, making it invisible in production.

## Findings

- Location: `crates/mika-gateway/src/github.rs`, line ~385
- The `let-chain` pattern `let Ok(mut cache) = state.github_delivery_cache.lock()` silently skips dedup on poisoned lock
- No warning or metric emitted when the lock is poisoned
- The comment says "fail-open for availability" but the condition is invisible

## Proposed Solutions

### Option 1: Log on poisoned lock (Recommended)
Add explicit match on the lock result to log a warning when poisoned.

**Pros:** Visibility into failure state, minimal code change
**Cons:** Slightly more verbose
**Effort:** Small
**Risk:** None

### Option 2: Use `parking_lot::Mutex` (no poisoning)
Replace `std::sync::Mutex` with `parking_lot::Mutex` which never poisons.

**Pros:** Eliminates the concern entirely
**Cons:** New dependency (though `parking_lot` is already in the dep tree via tokio)
**Effort:** Small
**Risk:** Low

## Acceptance Criteria

- [ ] Poisoned lock state is logged at warn level
- [ ] OR Mutex implementation that doesn't poison is used

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-04-02 | Created from code review of #382 | |
