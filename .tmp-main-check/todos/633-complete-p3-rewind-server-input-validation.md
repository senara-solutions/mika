---
status: pending
priority: p3
issue_id: "633"
tags: [code-review, security, rewind]
dependencies: []
---

# Add input validation to server rewind endpoints

## Problem Statement

The server rewind endpoints accept `session_id: String` and `after_message_id: i64` with no validation. A caller with `MIKA_INTERNAL_TOKEN` could send `after_message_id: 0` on an uncompacted session to delete all messages. Additionally, system/team sessions are not rejected.

## Findings

- **Source:** Security sentinel
- **Location:** `crates/mika-agent/src/server/rewind.rs` lines 9-16

## Proposed Solutions

Add validation:
1. `session_id` non-empty, length cap (1000 chars)
2. `after_message_id >= 0`
3. Reject rewind on `channel_type = 'system'` or `'team'` sessions (query sessions table)

- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Empty/oversized session_id returns 400
- [ ] Negative after_message_id returns 400
- [ ] System/team sessions return 422
- [ ] Tests for validation

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | |
