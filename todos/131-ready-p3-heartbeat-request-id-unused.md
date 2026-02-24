---
status: ready
priority: p3
issue_id: "131"
tags: [code-review, quality]
dependencies: []
---

# HeartbeatRequest.request_id Field Is Never Used

## Problem Statement

`HeartbeatRequest` has a `request_id: String` field, but the heartbeat handler binds it as `Json(_req)` with underscore prefix, never reading the value. Either the field should be used for logging/correlation, or the struct should be simplified.

## Findings

- **Source:** code-simplicity-reviewer
- **Location:** `crates/mika-agent/src/server/types.rs:29-31` and `handlers.rs:157`

## Proposed Solutions

### Option 1: Use request_id for heartbeat logging
- **Effort**: Trivial
- **Risk**: None

### Option 2: Remove field if truly unnecessary
- **Effort**: Trivial
- **Risk**: Low (breaking API change if gateway sends it)

## Acceptance Criteria

- [ ] Either request_id is used or removed
- [ ] All tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review

## Resources

- PR #5: Phase 2 Container HTTP Server
