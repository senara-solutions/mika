---
status: complete
priority: p2
issue_id: "112"
tags: [code-review, architecture, reliability]
dependencies: []
---

# DB Thread Panic Kills All Future Database Operations

## Problem Statement

If a closure sent to the `AsyncDatabase` background thread panics, the panic unwinds the thread, closing the `mpsc::Receiver`. All subsequent `with_db` calls will fail with "database thread has stopped". A single panicking operation takes down the entire database layer.

## Findings

- **Source:** architecture-strategist, performance-oracle
- **Location:** `crates/mika-agent/src/async_db.rs` lines 28-33
- **Evidence:** `std::thread::spawn(move || { while let Ok(f) = rx.recv() { f(&db); } })` — no panic isolation
- **Current impact:** Low in Phase 1 (closures are simple DB calls unlikely to panic)
- **Future impact:** Phase 2 with more complex operations increases panic risk

## Proposed Solutions

### Option 1: Wrap closure execution in catch_unwind (Recommended)
- **Pros**: Isolates panics, thread stays alive, caller gets a clear error
- **Cons**: `AssertUnwindSafe` wrapper needed (standard pattern)
- **Effort**: Small (3-5 lines)
- **Risk**: Low

```rust
std::thread::spawn(move || {
    while let Ok(f) = rx.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&db)));
    }
});
```

### Option 2: Store JoinHandle + auto-restart on panic
- **Pros**: Fully self-healing
- **Cons**: More complex, need to reopen DB connection
- **Effort**: Medium
- **Risk**: Medium (reopening connection has edge cases)

## Recommended Action

_To be filled during triage_

## Technical Details

- **Affected Files**: `crates/mika-agent/src/async_db.rs` (thread spawn)

## Acceptance Criteria

- [ ] A panicking closure does not kill the database thread
- [ ] Subsequent DB operations succeed after a panic
- [ ] Caller receives an error (not a hang) for the panicked operation
- [ ] Tests pass

## Work Log

### 2026-02-24 - Identified in v4 Code Review
**By:** Multi-agent review (architecture-strategist, performance-oracle)

## Resources

- Commit under review: 38a843b
- Rust docs: https://doc.rust-lang.org/std/panic/fn.catch_unwind.html
