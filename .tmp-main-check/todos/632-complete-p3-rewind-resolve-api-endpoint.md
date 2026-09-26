---
status: pending
priority: p3
issue_id: "632"
tags: [code-review, agent-native, rewind, api]
dependencies: []
---

# Add POST /api/v1/rewind/resolve endpoint for API parity

## Problem Statement

The TUI has `find_recent_exchanges` which resolves "undo last N exchanges" into `(session_id, anchor_message_id)` — including cross-session fallback. API callers must manually list sessions, fetch messages, walk trace_ids, and compute the anchor. This creates a context parity gap.

## Findings

- **Source:** Agent-native reviewer
- **Location:** `crates/mika-agent/src/server/rewind.rs` (no resolve endpoint exists)

## Proposed Solutions

### Option A: New resolve endpoint
`POST /api/v1/rewind/resolve` — wraps `find_recent_exchanges`.
Request: `{ "session_id": "...", "count": 1, "cross_session": false }`
Response: `{ "session_id": "...", "after_message_id": 42, "trace_ids": [...] }` or 404.
Behind `require_internal_token`.

- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Endpoint exists and returns resolved coordinates
- [ ] `cross_session` parameter controls fallback behavior
- [ ] OpenAPI spec updated

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | |
