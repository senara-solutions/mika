---
status: complete
priority: p2
issue_id: 643
tags: [code-review, architecture, database]
dependencies: []
---

# Add SESSION_MESSAGE_COLUMNS constant to prevent SQL column mismatch bugs

## Problem Statement

The `row_to_session_message` function expects a 9-column SELECT in a specific order, but there is no shared constant like `TASK_COLUMNS` to enforce it. The 9-column SELECT string is repeated across 12+ call sites. The root-cause bug fixed in this PR (missing `m.trace_id` in `get_messages_by_trace_id`) was a direct consequence of this repetition — one query out of 12 had a typo.

## Findings

- `TASK_COLUMNS` constant + `TASK_COLUMN_COUNT` already solves this for the `Task` struct (27 columns, centralized)
- `SessionMessage` has no equivalent — each call site manually writes `SELECT m.id, m.session_id, m.agent_id, m.role, m.content, s.channel_type, m.metadata, m.trace_id, m.created_at`
- 12+ queries use `row_to_session_message`: `load_recent_messages`, `load_conversation_summary`, `load_messages_before_window`, `load_messages_after`, `get_message_by_id`, `get_surrounding_messages`, `get_messages_since`, `get_messages_after_id`, `load_session_messages_paginated`, `get_messages_by_trace_id`, etc.
- Flagged by: architecture-strategist, performance-oracle, pattern-recognition-specialist

## Proposed Solutions

### Option A: Add SESSION_MESSAGE_COLUMNS constant (Recommended)
Add a `const SESSION_MESSAGE_COLUMNS: &str` similar to `TASK_COLUMNS`, and use it across all 12+ call sites.
- Pros: Eliminates entire class of column mismatch bugs, follows existing `TASK_COLUMNS` pattern
- Cons: Requires updating all 12+ call sites (mechanical change)
- Effort: Medium
- Risk: Low

### Option B: Keep as-is with comment convention
Add a comment above `row_to_session_message` documenting the expected column order.
- Pros: Zero code change
- Cons: Comments drift, doesn't prevent the bug class
- Effort: Small
- Risk: Medium (bug can recur)

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`
- **Pattern to follow:** `TASK_COLUMNS` / `TASK_COLUMN_COUNT` / `row_to_task` triple

## Acceptance Criteria

- [ ] `SESSION_MESSAGE_COLUMNS` constant defined alongside `row_to_session_message`
- [ ] All queries using `row_to_session_message` use the constant (with table alias prefix where needed)
- [ ] All existing tests pass
- [ ] No new queries can accidentally omit a column

## Work Log

- 2026-03-12: Created from code review of fix/trace-messages-endpoint branch
