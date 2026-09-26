---
status: complete
priority: p2
issue_id: "507"
tags: [code-review, agent-native, prompt-engineering, discoverability]
dependencies: []
---

# System Prompt Does Not Mention Task Tools — Agents Cannot Discover Callback Pattern

## Problem Statement

The system prompt mentions `create_reminder`, `list_reminders`, and `cancel_reminder` (prompt.rs lines 286-289) but has no reference to `create_task`, `cancel_task`, `list_tasks`, or the `callback/resume_agent` pattern. The agent will only discover these tools by scanning the tool list, not from the prompt's capability guidance section. The callback workflow is complex enough that it requires explanation.

## Findings

- **Source**: agent-native-reviewer (Warning)
- **Location**: `crates/mika-agent/src/prompt.rs:285-290`

The reminder block has no adjacent task block. An agent encountering `create_task` for the first time in the tool list has no guidance on:
- When to use tasks vs. reminders
- What `trigger_type` options are and their differences
- How the `callback/resume_agent` pattern works end-to-end
- That `mika ask --task-id` is how exec skills report back
- That the full UUID must be stored immediately after `create_task` (since `list_tasks` truncates it)

The callback/resume_agent pattern is the most powerful new capability but also the most complex. Without prompt guidance, the agent is unlikely to use it correctly or at all.

## Proposed Solutions

### Option A: Add "Scheduled Tasks" section to system prompt (Recommended)

Insert adjacent to the reminders block in `prompt.rs`:

```
**Scheduled Tasks** (`create_task`, `list_tasks`, `cancel_task`)

For time-based work, prefer `create_reminder`. Use `create_task` for:
- `trigger_type=callback`: Async background jobs where an external process delivers a result.
  Create the task, pass its UUID to the background process, then wait. When the process calls
  POST /tasks/{id}/complete (or mika ask --task-id), you will be re-run with the result injected.
  IMPORTANT: Store the returned UUID immediately — list_tasks only shows partial IDs.
- `trigger_type=recurring`: Scheduled skills (use cron expressions, 6-field format with seconds first).
- `trigger_type=time`: One-time future actions more complex than a simple reminder.
```

- **Effort**: Small | **Risk**: None (prompt addition only)

### Option B: Add inline documentation to tool definitions only

Tool descriptions in `definition()` already explain the fields. This avoids prompt bloat.

- **Effort**: None | **Risk**: Poor discoverability — agent must already be using the tool to read its description

## Acceptance Criteria

- [ ] System prompt includes a "Scheduled Tasks" section explaining when to use `create_task` vs. `create_reminder`
- [ ] Prompt explains the `callback/resume_agent` pattern including how external processes call back
- [ ] Prompt includes a note to store the UUID immediately after task creation
- [ ] Existing agent behavior for reminders is unaffected

## Work Log

- 2026-03-06: Identified by agent-native-reviewer of feat/unified-task-engine
