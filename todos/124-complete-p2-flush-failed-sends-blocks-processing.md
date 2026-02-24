---
status: complete
priority: p2
issue_id: "124"
tags: [code-review, performance]
dependencies: []
---

# flush_failed_sends Blocks Agent Processing

## Problem Statement

`flush_failed_sends` is called synchronously at the start of each message handler before the agent loop runs (`handlers.rs:87`). With up to 5 retries, each with a 10-second timeout + 2-second retry delay, worst case is ~60 seconds of blocking before the user's message is processed.

## Findings

- **Source:** performance-oracle (IMPORTANT-2)
- **Location:** `crates/mika-agent/src/server/handlers.rs:87` — `flush_failed_sends(&s).await`
- **Evidence:** Called inside the spawned task, before agent loop. Each send has 10s timeout + 2s retry = up to 12s per failed send, 5 sends = 60s worst case.

## Proposed Solutions

### Option 1: Move flush to a separate background task (not blocking agent)
- **Pros**: User message processed immediately, flush happens in parallel
- **Cons**: Slightly more complex, need to handle lock coordination
- **Effort**: Small
- **Risk**: Low

### Option 2: Reduce timeout and retry count for flush
- **Pros**: Keeps sequential simplicity, bounds worst case
- **Cons**: Still blocks, just less
- **Effort**: Trivial
- **Risk**: Low

### Option 3: Flush on heartbeat instead of on message
- **Pros**: Completely decouples from user message latency
- **Cons**: Longer delay before failed sends are retried
- **Effort**: Small
- **Risk**: Low

## Recommended Action

Option 1 — spawn flush as a separate task. The agent lock already serializes processing, so flush can run independently.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/server/handlers.rs`
- **Database Changes**: None

## Acceptance Criteria

- [ ] Failed send flush does not block agent message processing
- [ ] Failed sends are still retried reliably
- [ ] All tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** performance-oracle

## Resources

- PR #5: Phase 2 Container HTTP Server
