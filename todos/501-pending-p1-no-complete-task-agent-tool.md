---
status: pending
priority: p1
issue_id: "501"
tags: [code-review, agent-native, tools]
dependencies: []
---

# Agent Cannot Complete Its Own Callback Tasks — No `complete_task` Tool

## Problem Statement

The `POST /tasks/{id}/complete` endpoint and `mika ask --task-id` allow external processes and CLI users to deliver callback results. But the agent itself has no tool to mark a callback task complete and inject a result. This breaks the primary agent-native use case: an agent that spawned a background job cannot self-deliver the result without an external HTTP round-trip.

## Findings

- **Source**: agent-native-reviewer (Critical)
- **Location**: No `complete_task` tool file exists. HTTP handler: `crates/mika-agent/src/server/handlers.rs:299`

The `callback/resume_agent` architecture is well-designed end-to-end, but the agent-side tool is missing. An exec handler skill running in-process (e.g., `tmux`, `shell-exec`) that completes a background job must call back via HTTP to the agent's own server — there is no direct in-process path.

The callback flow requires:
1. Agent creates a `callback` task → gets UUID
2. Background work runs
3. **Result delivery** — currently only via `POST /tasks/{id}/complete` (HTTP) or `mika ask --task-id` (CLI)
4. `dispatch_completed_callback` → `dispatch_resume_agent` → agent resumes

Step 3 has no tool path. An agent orchestrating its own async work cannot close the loop without an external caller.

## Proposed Solutions

### Option A: Add `complete_task` tool (Recommended)

New file `crates/mika-agent/src/tools/complete_task.rs`:

```rust
pub struct CompleteTaskTool;

// Input schema: { "id": string (required), "result": string (required) }
// Logic:
// 1. Validate id (non-empty, ≤ 36 chars)
// 2. Validate result (non-empty, ≤ 100,000 chars)
// 3. Load task via ctx.db.get_task(id)
// 4. Check trigger_type == "callback"
// 5. Check status in ["pending", "in_progress"]
// 6. Call ctx.db.update_task_completed(id, Some(&result))
// 7. Log memory_event
// 8. Return success with task UUID
```

Register in `default_tools()` in `tools/mod.rs`.

Note: this tool does NOT trigger `dispatch_completed_callback` — that would create a re-entrant agent loop. The task is marked complete in the DB; the next engine tick or external observer sees the completion.

- **Effort**: Small | **Risk**: Low

### Option B: Accept current design, document the HTTP round-trip

Document that in-process skill callbacks must use the internal HTTP endpoint. Acceptable if exec handler skills always run as separate OS processes (which they do in the current shell-exec/tmux model).

- **Effort**: None | **Risk**: Parity gap remains for future in-process skill patterns

## Acceptance Criteria

- [ ] `complete_task` tool exists in `tools/complete_task.rs`
- [ ] Tool validates `id` (non-empty, ≤36 chars), `result` (non-empty, ≤100KB)
- [ ] Tool checks `trigger_type == "callback"` and `status in ["pending","in_progress"]`
- [ ] Tool is registered in `default_tools()`
- [ ] Tests cover: success, not-found, wrong trigger_type, already completed
- [ ] Agent can complete a callback task it created without HTTP round-trips

## Work Log

- 2026-03-06: Identified by agent-native-reviewer of feat/unified-task-engine
