---
status: complete
priority: p3
issue_id: "490"
tags: [code-review, agent-native, tools]
dependencies: []
---

# Agent Cannot Introspect All Task Types via list_reminders

## Problem Statement

`list_reminders` calls `get_pending_reminder_tasks()` which filters `action_type = 'send_message'`
only. The agent cannot see recurring heartbeat tasks (`run_skill`), `inject_context` tasks, or
any other task type. The user can see all tasks via the TUI `/tasks` command (which calls
`get_schedulable_tasks()` returning all types). This violates the agent-native parity principle:
the user has a view the agent cannot replicate.

## Findings

- **Source**: agent-native-reviewer review
- **Location**: `tools/list_reminders.rs:29–30`, `db.rs:890–898` (get_pending_reminder_tasks filters action_type = 'send_message')
- The `cancel_reminder` tool already calls `cancel_task` which works on any task type
- So the capability to act on any task exists, but discovery is restricted

## Proposed Solutions

### Option A: Add list_tasks tool mirroring TUI /tasks view (Recommended)
Create a new `list_tasks` tool that calls `get_schedulable_tasks()` (returns all pending tasks
regardless of action_type) and formats them similarly to the TUI `/tasks` display.
- **Effort**: Small | **Risk**: None

### Option B: Extend list_reminders to show all types
Add an `include_all: bool` parameter or remove the `action_type` filter.
- **Effort**: Tiny | **Risk**: Low (could confuse agents expecting only reminders)

## Acceptance Criteria

- [ ] Agent can see all pending tasks (heartbeat, reflection, send_message, etc.) via a tool
- [ ] The tool output matches what the user sees in the TUI `/tasks` view
- [ ] Existing `list_reminders` behavior for agents is preserved

## Work Log

- 2026-03-06: Identified by agent-native-reviewer of feat/unified-task-engine
