---
status: complete
priority: p2
issue_id: "123"
tags: [code-review, architecture, correctness]
dependencies: []
---

# Atomic Ordering Mismatch on Ready Flag

## Problem Statement

The `ready` flag in AppState uses `Ordering::Release` when stored (`mod.rs:103`) but `Ordering::Relaxed` when loaded (`handlers.rs:21`). The `Relaxed` load may not see the `Release` store on some architectures (e.g., ARM), causing the health endpoint to return 503 even after the server is ready.

## Findings

- **Source:** architecture-strategist (IMPORTANT-2)
- **Location:** `crates/mika-agent/src/server/mod.rs:103` — `ready.store(true, Ordering::Release)` and `crates/mika-agent/src/server/handlers.rs:21` — `state.ready.load(Ordering::Relaxed)`
- **Evidence:** Release/Relaxed is a valid pair only if there's an Acquire load to pair with the Release store. Relaxed provides no ordering guarantees.

## Proposed Solutions

### Option 1: Change load to Ordering::Acquire
- **Pros**: Correct Release/Acquire pairing, guaranteed visibility
- **Cons**: None (Acquire is cheap, especially on x86 where it's a no-op)
- **Effort**: Trivial (one word change)
- **Risk**: None

## Recommended Action

Option 1 — change `Ordering::Relaxed` to `Ordering::Acquire` in `handlers.rs:21`.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/server/handlers.rs`
- **Database Changes**: None

## Acceptance Criteria

- [ ] Health endpoint uses `Ordering::Acquire` for the ready flag load
- [ ] All tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** architecture-strategist

## Resources

- PR #5: Phase 2 Container HTTP Server
