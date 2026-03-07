---
status: complete
priority: p3
issue_id: "519"
tags: [code-review, patterns, observability]
dependencies: []
---

# `handle_task_complete` Error Responses Don't Echo Task ID — Inconsistent with `handle_message`

## Problem Statement

`handle_task_complete` error responses do not include the task UUID from the URL path. `handle_message` consistently echoes `request_id` in its 404 and 429 error response bodies. Callers logging responses cannot correlate task_complete errors back to the original task ID without parsing the error string.

## Findings

- **Source**: patterns-reviewer (F-2 Minor)
- **Location**: `crates/mika-agent/src/server/handlers.rs` — `handle_task_complete` error branches

Example from `handle_message` (consistent):
```json
{"error": "agent 'foo' not found", "request_id": "abc-123"}
```

`handle_task_complete` (inconsistent):
```json
{"error": "agent 'foo' not found"}
```

The task ID comes from the URL path extractor (`Path<String>`), not the body, but it is still available in the handler and should be included for correlation.

## Proposed Solutions

### Option A: Add `task_id` to all error response bodies (Recommended)

```rust
// Replace:
(StatusCode::NOT_FOUND, Json(json!({"error": format!("task '{}' not found", task_id)})))

// With:
(StatusCode::NOT_FOUND, Json(json!({"error": format!("task '{}' not found", task_id), "task_id": task_id})))
```

Apply consistently to all non-200 branches in `handle_task_complete`.

- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] All `handle_task_complete` error response bodies include `"task_id"` field
- [ ] Consistent with `handle_message` pattern of echoing the request identifier

## Work Log

- 2026-03-06: Identified by patterns-reviewer of feat/unified-task-engine
