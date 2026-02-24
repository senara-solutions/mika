---
status: pending
priority: p2
issue_id: "121"
tags: [code-review, performance]
dependencies: []
---

# reqwest::Client Created Per GatewayMessageSender Instantiation

## Problem Statement

`GatewayMessageSender::new()` calls `reqwest::Client::new()` each time it's instantiated. In server mode, a new sender is created for every message handler, heartbeat handler, and failed-sends flush — at least 3 `Client` instances per message turn. Each `reqwest::Client` allocates a new connection pool, DNS resolver, and TLS session cache.

## Findings

- **Source:** performance-oracle (IMPORTANT-1), architecture-strategist, code-simplicity-reviewer
- **Location:** `crates/mika-agent/src/messaging.rs:36` — `client: reqwest::Client::new()`
- **Evidence:** `GatewayMessageSender::new()` called in handlers.rs:92, handlers.rs:175, handlers.rs:212

## Proposed Solutions

### Option 1: Store shared reqwest::Client in AppState
- **Pros**: Single connection pool, reuses TCP connections, efficient
- **Cons**: Adds one more field to AppState
- **Effort**: Small
- **Risk**: Low

### Option 2: Use lazy_static / once_cell for a global client
- **Pros**: No AppState change
- **Cons**: Global state, harder to test
- **Effort**: Small
- **Risk**: Low

## Recommended Action

Option 1 — add `http_client: reqwest::Client` to AppState, pass to `GatewayMessageSender::new()`.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/messaging.rs`, `crates/mika-agent/src/server/state.rs`, `crates/mika-agent/src/server/handlers.rs`, `crates/mika-agent/src/server/mod.rs`
- **Database Changes**: None

## Acceptance Criteria

- [ ] Single reqwest::Client shared across all GatewayMessageSender instances
- [ ] All tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** performance-oracle, architecture-strategist, code-simplicity-reviewer

## Resources

- PR #5: Phase 2 Container HTTP Server
