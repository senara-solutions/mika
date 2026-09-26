---
status: pending
priority: p3
issue_id: 562
tags: [code-review, quality, observability]
dependencies: []
---

# Silent `let _ =` on critical task DB operations

## Problem Statement

Multiple task engine DB operations silently discard `Result` values using `let _ =`. This masks failures that could leave tasks stuck in limbo (e.g., permanently `in_progress`). The team task fix (PR fix/team-task-agent-id-mismatch) addressed two instances in `agent.rs`, but many more exist throughout the codebase.

## Findings

Locations with silent `let _ =` on task state mutations:

### Task Engine (`task_engine/engine.rs`)
- **Line 398**: `let _ = db.update_task_failed(...)` — recurring task reschedule error path
- **Lines 454-455**: `let _ = db.update_task_status(...)` and `let _ = db.update_task_next_fire_at(...)` — agent-busy retry
- **Line 444**: `let _ = db.update_task_failed(...)` — expired-while-busy path

### Dispatcher (`task_engine/dispatcher.rs`)
- **Line 276**: `let _ = self.db.mark_task_delivered(...)` — could cause duplicate TUI delivery
- **Line 537**: `let _ = self...` — another silenced result

### Skills Executor (`skills/executor.rs`)
- **Lines 556, 590, 614**: `let _ = db.update_task_failed(...)` — spawn/wait/exit failure paths
- **Line 565**: `let _ = db.set_task_process_id(...)` — PID recording

### Server Handlers (`server/handlers.rs`)
- **Lines 418-419, 434-448**: Multiple `let _ =` on fallback error handling paths

## Proposed Solutions

### Option A: match + warn pattern (Recommended)
Replace each `let _ =` with `match` + `warn!` logging, as done in `agent.rs` by the team task fix.

- Pros: Consistent with existing pattern, adds observability
- Cons: Minor code expansion
- Effort: Small
- Risk: None

## Acceptance Criteria

- [ ] All `let _ =` on task state mutation methods replaced with match + warn
- [ ] No new clippy warnings
- [ ] Existing tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Pattern recognition agent identified 12+ instances |
