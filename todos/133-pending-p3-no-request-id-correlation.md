---
status: pending
priority: p3
issue_id: "133"
tags: [code-review, observability]
dependencies: []
---

# No Request ID Correlation in Async Processing Logs

## Problem Statement

When a message is accepted (202), the `request_id` is logged at accept time and completion time, but not propagated to intermediate agent loop logs. This makes it difficult to correlate logs for a specific request during debugging.

## Findings

- **Source:** agent-native-reviewer
- **Location:** `crates/mika-agent/src/server/handlers.rs` — request_id only in outer logs

## Proposed Solutions

### Option 1: Add request_id to tracing span for the spawned task
- **Pros**: All child logs automatically tagged
- **Cons**: Requires tracing span setup
- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] All logs within a message processing task include the request_id
- [ ] tracing span established for spawned task

## Work Log

### 2026-02-24 - Identified during PR #5 review

## Resources

- PR #5: Phase 2 Container HTTP Server
