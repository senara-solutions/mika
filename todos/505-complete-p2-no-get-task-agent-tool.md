---
status: complete
priority: p2
issue_id: "505"
tags: [code-review, agent-native, tools]
dependencies: []
---

# No `get_task` Tool — Agent Cannot Inspect a Specific Task by ID

## Problem Statement

`db.get_task()` is used in the HTTP handler but there is no agent tool exposing it. An agent that creates a callback task and stores its ID in memory has no way to check whether it has been completed, what its current status is, or what result was delivered — without relying on `list_tasks` which only shows 8-character ID prefixes and excludes completed/failed/expired tasks.

## Findings

- **Source**: agent-native-reviewer (Warning)
- **Location**: No `get_task` tool file exists. `db.get_task()` at `crates/mika-agent/src/db.rs:767`

`list_tasks` (list_tasks.rs:50) truncates IDs to 8 chars (`&t.id[..8.min(t.id.len())]`). This makes it impossible to:
- Verify a task was completed (status `completed` tasks don't appear in `list_tasks`)
- Retrieve the `result` field stored by the completing process
- Cross-reference a stored full UUID against the list

An agent in a `callback/resume_agent` workflow needs to be able to check task status proactively, especially when handling errors or timeouts.

## Proposed Solutions

### Option A: Add `get_task` tool (Recommended)

New file `crates/mika-agent/src/tools/get_task.rs`:

```rust
pub struct GetTaskTool;

// Input schema: { "id": string (required, full UUID) }
// Returns all fields: id, label, status, trigger_type, action_type,
//                     next_fire_at, timeout_at, result, created_at
```

Register in `default_tools()`.

Output format (JSON-like text):
```
Task: <full-uuid>
Label: <label>
Status: <status>
Trigger: <trigger_type>
Action: <action_type>
Created: <timestamp>
Timeout: <timestamp or "none">
Next fire: <timestamp or "n/a">
Result: <result or "none">
```

- **Effort**: Small | **Risk**: None

## Acceptance Criteria

- [ ] `get_task` tool exists in `tools/get_task.rs`
- [ ] Tool accepts a full UUID and returns all task fields including `status` and `result`
- [ ] Tool is registered in `default_tools()`
- [ ] Returns clear error if task not found
- [ ] Tests cover: success (pending), success (completed with result), not-found

## Work Log

- 2026-03-06: Identified by agent-native-reviewer of feat/unified-task-engine
