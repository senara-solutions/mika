---
title: "fix(gateway): remove bot self-event filter"
type: fix
status: completed
date: 2026-04-02
---

# Remove bot self-event filter from gateway webhook handler

## Overview

The bot self-event filter in `crates/mika-gateway/src/github.rs` drops ALL webhook events from `mika-dev-bot[bot]`. With both mika-dev and mika-qa sharing the same GitHub App identity, this blocks the entire webhook-driven dev loop — mika-qa never receives PR events, and mika-dev never receives review verdicts.

## Problem Statement

The filter was designed for a single-agent scenario to prevent infinite loops. Now that two agents (mika-dev, mika-qa) share one GitHub App identity but subscribe to disjoint event types, the filter incorrectly blocks legitimate cross-agent communication.

## Proposed Solution

Remove the `is_bot_self_event()` filter call and all supporting code from the gateway. Loop prevention is guaranteed by the routing table — no event type routes to both agents.

## Acceptance Criteria

- [x] Bot self-event filter call removed from `handle_github_webhook()`
- [x] `github_app_id` field removed from gateway `AppState` and `GatewaySettings`
- [x] `is_bot_self_event()` function and its tests removed
- [x] Explanatory comment added at the former filter location
- [x] Compound solution doc updated to reflect removal
- [x] `MIKA_GITHUB_APP_ID` env var still works for agent-side JWT auth (untouched)
- [x] All gateway tests pass (111 passed)
- [x] Clippy clean

## MVP

### `crates/mika-gateway/src/github.rs`

Remove the filter call (lines 398-402) and replace with comment:

```rust
// Self-event filter intentionally removed: mika-dev and mika-qa share one GitHub
// App identity but subscribe to disjoint event types. No routing path exists that
// would deliver an agent's own action back to itself. Loop prevention is guaranteed
// by the routing table, not by identity filtering.
// Future: give mika-qa a dedicated App token (Option 3) if per-agent audit trails
// or permission scopes become necessary.
```

Remove `is_bot_self_event()` function (lines 160-176).

Remove `make_event()` helper, 6 unit tests (lines 682-764), and integration test `test_webhook_bot_self_event_filtered` (lines 1052-1066).

### `crates/mika-gateway/src/routes.rs`

Remove `github_app_id: Option<u64>` field from `AppState` struct and its Debug impl line.

### `crates/mika-gateway/src/settings.rs`

Remove `github_app_id: Option<u64>` field from `GatewaySettings` and its Debug line. Update test fixture.

### `crates/mika-gateway/src/main.rs`

Remove `github_app_id` from AppState construction and its startup log block.

### `docs/solutions/architecture-patterns/github-webhook-endpoint-gateway.md`

Update to reflect that design decision #4 (bot self-event filtering) has been removed, with rationale.

## Sources

- Related issue: #401
- Gateway webhook handler: `crates/mika-gateway/src/github.rs`
- GitHub App JWT module (untouched): `crates/mika-common/src/github_app.rs`
- Compound doc: `docs/solutions/architecture-patterns/github-webhook-endpoint-gateway.md`
