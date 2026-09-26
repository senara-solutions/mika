---
status: pending
priority: p3
issue_id: 759
tags: [code-review, dashboard, agent-native]
dependencies: []
---

# Dashboard API `MessageResponse` missing `internal` field

## Problem Statement

The `MessageResponse` struct in `crates/mika-agent/src/server/dashboard.rs` does not include the `internal: bool` field from `SessionMessage`. When messages are returned via the dashboard API (`/api/v1/sessions/:id/messages`), the internal flag is silently dropped.

## Findings

- `SessionMessage` gained `internal: bool` in schema v22 (#494)
- The `From<SessionMessage>` impl for `MessageResponse` does not map the field
- Dashboard and API consumers cannot distinguish internal from user-facing messages
- The plan for #494 explicitly defers dashboard API filtering to a follow-up issue

## Proposed Solutions

### Option A: Add field to MessageResponse (recommended)
Add `pub internal: bool` to `MessageResponse` and map it in the `From` impl. Optional: add `?exclude_internal=true` query parameter to message endpoints.

- **Pros:** Simple, preserves information, backward-compatible (new JSON field)
- **Cons:** None significant
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] `MessageResponse` includes `internal: bool`
- [ ] API responses include the `internal` field in JSON
