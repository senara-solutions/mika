---
status: pending
priority: p2
issue_id: 712
tags: [code-review, performance]
dependencies: []
---

# N+1 redundant session_id lookup in a2a_build_task

## Problem Statement
`a2a_build_task` performs 3-4 sequential queries: (1) joins a2a_task_map + tasks, (2) a2a_get_messages which re-queries a2a_task_map for session_id (redundant), (3) a2a_get_artifacts. The session_id from query 1 is already available but not passed through.

## Findings
- `crates/mika-agent/src/a2a_db.rs` lines 446-512
- Query 1 (line 452-458) joins a2a_task_map but doesn't select session_id
- `a2a_get_messages` (line 194-202) re-queries a2a_task_map for session_id

## Proposed Solutions
Select `m.session_id` in the first query and pass it to a `a2a_get_messages_by_session(&session_id, limit)` variant, eliminating the redundant lookup.

## Acceptance Criteria
- [ ] `a2a_build_task` uses at most 3 queries (no redundant session_id lookup)
