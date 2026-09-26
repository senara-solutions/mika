---
title: "fix: Deliver failed callback tasks to agent"
type: fix
status: completed
date: 2026-03-18
issue: 203
---

# fix: Deliver failed callback tasks to agent

## Overview

When a long-running exec handler exits with a non-zero code, the background monitor marks the task as `status = 'failed'` in the DB. However, the agent is **never notified** because all callback delivery paths only query for `status = 'completed'`. The agent waits indefinitely for a callback that will never arrive.

## Problem Statement

Four code locations hardcode `status = 'completed'`, creating a blind spot for failed tasks:

1. `db.rs:2477` — `get_undelivered_callback_tasks` SQL WHERE clause
2. `db.rs:2503` — `get_undelivered_callback_tasks_for_session` SQL WHERE clause
3. `db.rs:2522` — `mark_task_delivered` SQL UPDATE filter
4. `app.rs:1353` — TUI system message always says "completed"

Additionally:
- Server-mode dispatcher (`dispatcher.rs`) calls the same `mark_task_delivered`, so it inherits the bug
- The TUI error-recovery path in `chat.rs:352` hardcodes reset to `'completed'`, which would corrupt a failed task's original status
- `format_callback_framing` in `agent.rs` says "A background task has completed" for all callbacks

## Proposed Solution

### Phase 1: DB Layer — Widen Status Filters

**`crates/mika-agent/src/db.rs`**

1. **`get_undelivered_callback_tasks`** (~line 2477): Change `status = 'completed'` to `status IN ('completed', 'failed')`
2. **`get_undelivered_callback_tasks_for_session`** (~line 2503): Same change
3. **`mark_task_delivered`** (~line 2522): Change `WHERE ... AND status = 'completed'` to `WHERE ... AND status IN ('completed', 'failed')`
4. **Partial index** `idx_tasks_callback_delivery`: Broaden predicate to include `'failed'` — update in the latest schema creation path (pre-1.0, no migration needed; fresh DBs get the new index)

### Phase 2: TUI — Handle Failed Callbacks

**`crates/mika-cli/src/tui/app.rs`** — `poll_callback_tasks` (~line 1319):

- Check `task.status` to determine if completed or failed
- Show system message `[{label}] failed` for failed tasks (instead of always "completed")
- For failed tasks with empty/null result, use fallback: `"Task failed with no error details."`
- Pass `task.status` through `AgentRequest::CallbackResult` so error-recovery can restore the original status

**`crates/mika-cli/src/commands/chat.rs`** — error-recovery path (~line 352):

- Thread the original task status through `AgentRequest::CallbackResult` (add field)
- On agent failure, reset to the original status (`"completed"` or `"failed"`) instead of hardcoding `"completed"`

### Phase 3: Agent Framing — Distinguish Success from Failure

**`crates/mika-agent/src/agent.rs`** — `format_callback_framing`:

- Accept a `failed: bool` parameter (or a status string)
- When failed: use "A background task has failed" preamble with instruction to report the error
- The `<callback_result trust="untrusted">` wrapper stays the same — the agent distinguishes from the framing text and result content

### Phase 4: Server Mode — Dispatch Failed Callbacks

**`crates/mika-agent/src/task_engine/dispatcher.rs`** — `dispatch_resume_agent`:

- The dispatcher already works if it receives a failed task — it reads `task.result` and calls `run_silent_agent`
- The fix is upstream: the task engine's DB scan must include failed callback tasks so they enter the dispatch queue
- In `engine.rs`, add a periodic scan for `status = 'failed' AND trigger_type = 'callback' AND action_type = 'resume_agent'` tasks (similar to TUI polling)
- After successful silent agent run, call `mark_task_delivered` (now accepts failed tasks)

### Phase 5: Tests

**`crates/mika-agent/src/db.rs`** — add tests:

- `test_get_undelivered_callback_tasks_returns_failed_tasks` — failed tasks with `completed_at` are returned
- `test_get_undelivered_callback_tasks_returns_both_completed_and_failed` — mixed results ordered by `completed_at`
- `test_mark_task_delivered_claims_failed_task` — failed task transitions to delivered
- `test_mark_task_delivered_failed_then_no_double_claim` — second claim on delivered-from-failed is rejected

## Technical Considerations

### State Machine

The task status transitions become:
```
pending → in_progress → completed → delivered
                      → failed    → delivered
```

Both `completed` and `failed` are now "deliverable" terminal states before `delivered`.

### Loop Prevention Invariants (must maintain)

Per `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`:
- `is_callback_turn: true` — disables `LongRunningContext` (prevents spawning new long-running tasks from callback)
- `<callback_result trust="untrusted">` wrapping — maintained for both completed and failed
- `safe_always_on_skills()` — only builtin handlers in silent/callback mode
- 100KB result size limit — applies to failed task results too

### Error-Recovery Status Preservation

The `AgentRequest::CallbackResult` variant needs an additional field to carry the original status:

```rust
// In the enum that defines AgentRequest (or equivalent)
CallbackResult {
    task_id: String,
    label: String,
    result: String,
    original_status: String,  // NEW: "completed" or "failed"
}
```

### Server-Mode Delivery

The task engine tick loop already scans for pending tasks. Adding a scan for failed callbacks follows the same pattern. The scan should:
- Run at the same frequency as the existing DB scan (every 60 ticks)
- Use the same `get_undelivered_callback_tasks` method (now returns failed tasks too)
- Dispatch via `dispatch_resume_agent` with the failed status context

### Out of Scope

- **Race between monitor and external completion** (`update_task_failed` has no status guard) — pre-existing, separate issue
- **Team engine grandchild failure notification** — separate issue; `count_pending_callback_tasks_by_team_run` correctly counts only pending tasks
- **Dashboard display changes** — dashboard already handles `failed` status; `delivered` following `failed` is transparent
- **Retry limits for failed callback delivery** — matches current completed-callback behavior (no limit)
- **Index migration for existing DBs** — pre-1.0, fresh deploys get the updated index

## Acceptance Criteria

- [x] When a long-running handler exits non-zero, the TUI agent receives the failure as a callback within ~5s
- [x] The callback result contains the error message (exit code + stderr)
- [x] The agent can distinguish success from failure via `format_callback_framing` text
- [x] Existing completed callback delivery is unchanged
- [x] Server mode also delivers failed callbacks via task engine scan
- [x] TUI error-recovery preserves original task status (doesn't corrupt failed → completed)
- [x] Failed tasks with empty results get a fallback message instead of being silently dropped
- [x] Tests cover: failed task polling, mixed status returns, failed task claiming, double-claim rejection

## Sources & References

- GitHub issue: [#203](https://github.com/senara-solutions/mika/issues/203)
- Callback TUI delivery pattern: `docs/solutions/architecture-patterns/callback-tui-delivery-polling.md`
- Loop prevention guards: `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`
- Callback lifecycle: `docs/solutions/architecture/callback-resume-agent-lifecycle.md`

### Key Files

| File | Lines | Change |
|------|-------|--------|
| `crates/mika-agent/src/db.rs` | 2477, 2503, 2522 | Widen status filters |
| `crates/mika-agent/src/db.rs` | 1216, 1571 | Broaden partial index predicate |
| `crates/mika-cli/src/tui/app.rs` | 1319-1369 | Handle failed tasks in poll |
| `crates/mika-cli/src/commands/chat.rs` | 349-352 | Preserve original status in error recovery |
| `crates/mika-agent/src/agent.rs` | 48-62 | Differentiate callback framing |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | 308-328 | Server-mode delivery |
| `crates/mika-agent/src/task_engine/engine.rs` | scan loop | Add failed callback scan |
| `crates/mika-agent/src/db.rs` | tests section | New test cases |
