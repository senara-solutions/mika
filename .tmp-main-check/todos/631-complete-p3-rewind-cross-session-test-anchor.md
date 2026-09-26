---
status: pending
priority: p3
issue_id: "631"
tags: [code-review, testing, rewind]
dependencies: []
---

# Add anchor_id verification to cross-session rewind test

## Problem Statement

`test_find_recent_exchanges_cross_session` does not verify the returned `anchor_id`. The existing `test_find_recent_exchanges` verifies it indirectly by calling `get_messages_after_id` with the anchor. The cross-session test should do the same.

## Findings

- **Source:** Pattern recognition specialist
- **Location:** `crates/mika-agent/src/rewind.rs` test at ~line 1094

## Proposed Solutions

Add `get_messages_after_id("test-session", anchor_id)` assertion to verify the anchor produces the expected message range.

- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Test verifies anchor_id produces correct message set via `get_messages_after_id`

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | |
