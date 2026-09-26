---
title: "Fix trace messages endpoint — missing column, silent errors, lost trace IDs"
date: 2026-03-12
module: mika-agent (db, server, CLI), dashboard
tags: [bug, dashboard, trace, sqlite, column-mismatch, error-handling, observability]
severity: high
symptoms:
  - "Message 685 has no trace_id in database"
  - "Trace f221c4ea... shows nothing in Trace Detail view"
  - "Trace e5e8996e... shows audit logs instead of expected messages"
  - "Trace 64834561... shows no message when it should show message 677"
root_cause: >
  get_messages_by_trace_id() SELECT listed 8 columns but row_to_session_message()
  expects 9 (m.trace_id was missing), causing a rusqlite column-count error on every
  call. The dashboard swallowed the resulting HTTP error because TraceDetail.tsx
  discarded the messagesError from the query hook. Additionally, CLI callback
  tool_result messages were saved without propagating the task's created_trace_id,
  so those messages could never be found by trace. The Task struct also omitted the
  created_trace_id field that exists in the DB, preventing trace_id propagation
  through the callback pipeline.
resolution: >
  Introduced SESSION_MESSAGE_COLUMNS constant to centralize the 9-column SELECT list
  across all 10+ message queries, eliminating column-count drift. Added created_trace_id
  to the Task struct and adjusted column ordinals. Propagated trace_id through
  AgentRequest::CallbackResult into save_message_with_metadata. Surfaced messagesError in
  TraceDetail.tsx. Added unit and integration tests for the fixed paths.
---

# SQL Column Mismatch Breaks Trace Detail View

## Problem

The dashboard's "Trace Detail" view was completely non-functional for displaying messages. Four symptoms were reported:

1. Message 685 had no `trace_id` in the database
2. Trace `f221c4ea...` showed nothing
3. Trace `e5e8996e...` showed audit logs instead of expected messages
4. Trace `64834561...` showed no message when it should show message 677

## Root Cause Analysis

The `get_messages_by_trace_id()` query in `crates/mika-agent/src/db.rs` selected **8 columns** but was passed to `row_to_session_message()`, which reads **9 columns** by positional index (0 through 8).

The query was:
```sql
SELECT m.id, m.session_id, m.agent_id, m.role, m.content, s.channel_type, m.metadata, m.created_at
```

Missing: `m.trace_id` at index 7.

The `row_to_session_message` mapper expects:
- 0: `id`, 1: `session_id`, 2: `agent_id`, 3: `role`, 4: `content`, 5: `channel_type`, 6: `metadata`, **7: `trace_id`**, 8: `created_at`

**Cascade failure:** Because `m.trace_id` was omitted, `m.created_at` (an integer timestamp) landed at index 7 where `trace_id` (`Option<String>`) was expected. SQLite's dynamic typing allowed the integer-to-string coercion to silently succeed. However, `r.get::<_, i64>(8)` for `created_at` was out of bounds (only 8 columns, indices 0-7), causing a **runtime error on every call**. The dashboard endpoint returned 500 errors, and `TraceDetail.tsx` silently swallowed them because it only checked `eventsError`, not `messagesError`.

Three secondary issues compounded the problem:

- **`Task` struct missing `created_trace_id`**: The field existed in the DB schema (added in v4-v5 migration) and in `NewTask`, but `Task` (the read-back struct) and `row_to_task` omitted it, causing all subsequent column indices to be off by one.
- **CLI callback trace_id not propagated**: `AgentRequest::CallbackResult` had no `trace_id` field, so callback result messages were saved with `trace_id: None`, making them invisible in trace views.
- **Duplicated column lists**: The 9-column SELECT was hand-written across 10+ queries with no shared constant, making it easy for one query to drift.

## Fixes Applied

### 1. SQL column fix (`db.rs`)

All queries using `row_to_session_message` now reference `SESSION_MESSAGE_COLUMNS`:

```rust
const SESSION_MESSAGE_COLUMNS: &'static str =
    "m.id, m.session_id, m.agent_id, m.role, m.content, s.channel_type, m.metadata, m.trace_id, m.created_at";
```

This follows the existing `TASK_COLUMNS` pattern. 10 queries updated.

### 2. `Task` struct `created_trace_id` (`db.rs`)

```rust
pub struct Task {
    // ...
    pub created_by_session: Option<String>,
    pub created_trace_id: Option<String>,  // added
    pub created_at: i64,
    // ...
}
```

`row_to_task` updated with `created_trace_id` at index 20, shifting subsequent fields. `TASK_COLUMNS` and `TASK_COLUMN_COUNT` (26 -> 27) updated.

### 3. CLI callback trace_id threading (`app.rs`, `chat.rs`)

```rust
// app.rs — enum variant
AgentRequest::CallbackResult {
    task_id: String,
    label: String,
    result: String,
    trace_id: Option<String>,  // added
}

// app.rs — dispatch site
trace_id: task.created_trace_id,  // propagate from task

// chat.rs — message save
save_message_with_metadata(..., trace_id.as_deref())  // was: None
```

### 4. Dashboard error surfacing (`TraceDetail.tsx`)

```tsx
const { data: messages, isLoading: messagesLoading, error: messagesError } = useTraceMessages(...)
const error = eventsError || messagesError  // was: eventsError only
```

### 5. Clarifying comment (`db.rs`)

Added `// no trace_id — summaries span multiple traces` on `replace_with_summary` to prevent future "fixes".

## Prevention Strategies

### Column constant pattern

`SESSION_MESSAGE_COLUMNS` centralizes the column list. Any future column changes are single-point edits. The constant and mapper live adjacent in the code, making drift obvious during review. This mirrors `TASK_COLUMNS` which already existed for `row_to_task`.

### Testing strategy

Every new `pub` DB function returning a struct must have a roundtrip test. The missing test for `get_messages_by_trace_id` allowed this bug to ship. Two tests were added:

- `test_get_messages_by_trace_id` in `db.rs` — unit test verifying query + deserialization
- `test_trace_messages_returns_matching_messages` in `server/mod.rs` — integration test for the HTTP endpoint

### Code review checklist

When reviewing DB query changes:

- [ ] Does the SELECT use the column constant (`SESSION_MESSAGE_COLUMNS`, `TASK_COLUMNS`)?
- [ ] If hand-written, does it use a different mapper?
- [ ] Is there a roundtrip test?
- [ ] If a column was added/removed, was `*_COLUMN_COUNT` updated?

### Future consideration: named column access

`row.get("column_name")` instead of `row.get(N)` would eliminate positional mismatch entirely. `rusqlite` supports this. Recommended for new mappers; migrate existing ones opportunistically.

## Test Coverage

- `test_get_messages_by_trace_id` — insert message with trace_id, verify roundtrip, verify empty for nonexistent trace
- `test_trace_messages_returns_matching_messages` — HTTP endpoint returns 200 with correct message data

## Related Documentation

- [trace-id-correlation-unified-observability.md](../architecture-patterns/trace-id-correlation-unified-observability.md) — Trace_id propagation architecture (v4-v5 schema migration)
- [sqlite-datetime-format-mismatch.md](./sqlite-datetime-format-mismatch.md) — Similar class of silent query failure due to column type assumptions
- [callback-resume-agent-lifecycle.md](../architecture/callback-resume-agent-lifecycle.md) — Callback task lifecycle and delivery paths
- [callback-tui-delivery-polling.md](../architecture-patterns/callback-tui-delivery-polling.md) — TUI callback polling mechanism
- [team-task-child-wrong-agent-id.md](./team-task-child-wrong-agent-id.md) — Related pattern: column/query mismatch causing silent failures
