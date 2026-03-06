---
status: pending
priority: p2
issue_id: "484"
tags: [code-review, agent-native, tools]
dependencies: []
---

# cancel_reminder Requires Full UUID but list_reminders Shows 8-Character Prefix

## Problem Statement

`list_reminders` and `create_reminder` output task IDs as 8-character prefixes
(e.g., `a3f7c291`). `cancel_reminder` passes the ID to `cancel_task` in the DB layer, which
matches `WHERE id = ?1` — requiring an exact full UUID. When the agent calls
`cancel_reminder({"id": "a3f7c291"})`, the cancellation silently returns "not found" because
the short ID doesn't match any full UUID. The agent cannot cancel reminders it has just created.
The `/tasks cancel <id>` TUI command has the same issue (shows 12-char prefix, DB needs full UUID).

## Findings

- **Source**: agent-native-reviewer review
- **Location**: `tools/list_reminders.rs:38–43`, `tools/create_reminder.rs:99,114`, `tools/cancel_reminder.rs:33–39`, `db.rs:848–853`
- `cancel_task` at `db.rs:848` uses `WHERE id = ?1` — exact UUID match only
- The agent cannot independently cancel any reminder it has created, which is a complete
  agent-native parity failure for the reminder lifecycle

## Proposed Solutions

### Option A: Use full UUIDs in all tool output (Recommended)
Change `list_reminders` and `create_reminder` to output the full UUID instead of the 8-char prefix.
UUIDs are readable enough in tool output and are already used in the DB.
- **Pros**: Simplest fix, no DB change needed, self-consistent
- **Effort**: Tiny | **Risk**: None

### Option B: Add prefix-expansion in cancel_task
Add `WHERE id LIKE ?1 || '%'` or a two-step lookup (find by prefix, then cancel by full ID).
- **Pros**: Works with existing short IDs in tool history
- **Cons**: Prefix ambiguity if two UUIDs share prefix, more complex query
- **Effort**: Small | **Risk**: Low

## Acceptance Criteria

- [ ] Agent can cancel a reminder using the ID shown in `list_reminders` / `create_reminder` output
- [ ] `create_reminder` success message includes the ID that `cancel_reminder` accepts
- [ ] Round-trip test: create reminder → list → cancel by listed ID → verify cancelled

## Work Log

- 2026-03-06: Identified by agent-native-reviewer of feat/unified-task-engine
