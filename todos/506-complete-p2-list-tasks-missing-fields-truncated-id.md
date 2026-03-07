---
status: complete
priority: p2
issue_id: "506"
tags: [code-review, agent-native, tools, usability]
dependencies: []
---

# `list_tasks` Shows 8-Char UUID Prefix — Cannot Cancel Task from List Alone

## Problem Statement

`list_tasks` output truncates task IDs to 8 characters and omits `trigger_type` and `timeout_at`. An agent that finds a task via `list_tasks` cannot directly use that ID with `cancel_task` (which requires a full UUID), cannot determine whether a task is a callback vs. time-triggered, and cannot see if a timeout is pending.

## Findings

- **Source**: agent-native-reviewer (Warning)
- **Location**: `crates/mika-agent/src/tools/list_tasks.rs:50-58`

Current output format:
```
- {short_id} [{status}] {label} ({action_type}) — next: {fire_at}
```

Where `short_id = &t.id[..8.min(t.id.len())]`.

Consequences:
1. Agent cannot use the 8-char prefix with `cancel_task` → must already know the full UUID
2. No `trigger_type` shown → agent cannot distinguish callback tasks from time tasks
3. No `timeout_at` shown → agent doesn't know if a callback is about to expire

The patterns-reviewer also confirmed that the 8-char truncation is internally inconsistent: `cancel_task` requires a full UUID, so `list_tasks` output is not actionable for cancellation without a separate `get_task` lookup.

## Proposed Solutions

### Option A: Show full UUID + add trigger_type + timeout_at (Recommended)

Updated output format:
```
- {full_uuid} [{status}] {label}
  Trigger: {trigger_type} | Action: {action_type}
  Next: {fire_at} | Timeout: {timeout_at or "none"}
```

Or single-line with more columns:
```
- {full_uuid} [{status}/{trigger_type}] {label} ({action_type}) — next: {fire_at} timeout: {timeout_at}
```

- **Effort**: Tiny | **Risk**: None (output format change only)

### Option B: Show 16-char prefix as compromise

Shows enough characters to be visually distinctive while keeping lines shorter.

- **Effort**: Tiny | **Risk**: Still not usable with cancel_task (needs full UUID)

## Acceptance Criteria

- [ ] `list_tasks` output includes the full task UUID (not 8-char prefix)
- [ ] Output includes `trigger_type` for each task
- [ ] Output includes `timeout_at` if set
- [ ] Existing `list_tasks` tests updated to match new format
- [ ] An agent that lists tasks can directly use the ID with `cancel_task`

## Work Log

- 2026-03-06: Identified by agent-native-reviewer of feat/unified-task-engine
