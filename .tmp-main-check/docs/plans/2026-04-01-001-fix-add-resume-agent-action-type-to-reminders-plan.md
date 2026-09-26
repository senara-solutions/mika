---
title: "fix: add resume_agent action type to reminders"
type: fix
status: active
date: 2026-04-01
issue: 363
---

# fix: add resume_agent action type to reminders

## Overview

`create_reminder` hardcodes `action_type: "send_message"`, which only sends a Telegram notification. In CLI mode (no `MessageSender`), reminders are silently dropped. CI follow-up reminders ("re-check in 5 min") never wake the agent to check CI and merge.

Add `resume_agent` as a supported `action_type` for reminders. When a `resume_agent` reminder fires, it triggers a silent agent turn with the reminder message as context — the agent can then check CI, merge PRs, or take any tool-based action.

## Problem Statement

During the mika#359 dev run, mika-dev created a 5-min CI follow-up reminder. The reminder fired (status: completed at 20:38:47) but dispatched `send_message` — no agent loop, no CI check, no merge. PR #362 sat with green CI waiting indefinitely.

Root cause: `create_reminder.rs` line 244 hardcodes `action_type: "send_message"`. `dispatch_send_message` (dispatcher.rs:121-145) silently drops when no `MessageSender` is configured.

## Proposed Solution

Reuse the existing `dispatch_resume_agent` path with minimal adaptations to handle the reminder lifecycle (which differs from the callback lifecycle).

## Technical Approach

### Critical: Task Lifecycle Mismatch

The main architectural challenge is that `dispatch_resume_agent` was designed for **callback** tasks, which have a different lifecycle than **reminder** tasks:

| Aspect | Callback lifecycle | Reminder lifecycle |
|--------|-------------------|-------------------|
| Status at dispatch time | `completed` or `failed` | `in_progress` (claimed by `fire_task`) |
| Context source | `task.result` (set by external process) | `action_config.text` (set at creation) |
| Post-dispatch | `mark_task_delivered()` | `fire_task()` marks `completed` |
| Session prefix | `callback-{uuid}` | Should be `reminder-{uuid}` |
| Trust level | Untrusted (external data) | Trusted (user-authored message) |
| Prompt framing | "A background task completed" | "A scheduled reminder fired" |

### Implementation Steps

#### 1. Add `SilentTrigger::Reminder` variant

**File:** `crates/mika-agent/src/agent.rs`

Add a new variant to `SilentTrigger`:

```rust
Reminder {
    task_id: String,
    message: String,
},
```

Add prompt framing in `build_silent_prompt` / `run_silent_inner`:
- Framing: "A reminder you set has fired. Here is what you scheduled yourself to do:"
- The message is NOT wrapped in `<callback_result trust="untrusted">` — it's a user-authored reminder, not external data
- Task health context: Include (same as Heartbeat/Callback) so the agent has work item awareness

#### 2. Modify `dispatch_resume_agent` to handle both lifecycles

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

Refactor `dispatch_resume_agent` to detect whether the task is a callback or a reminder:

```rust
// Determine context source based on trigger_type
let (context, is_callback, is_failed) = if task.trigger_type == "callback" {
    // Existing callback path: read from task.result
    let is_failed = task.status == "failed";
    let result = match task.result.clone() {
        Some(r) if !r.is_empty() => r,
        _ if is_failed => FAILED_TASK_FALLBACK.to_string(),
        _ => return Err(anyhow!("no result")),
    };
    (result, true, is_failed)
} else {
    // Reminder path: read from action_config.text
    let config: serde_json::Value = serde_json::from_str(&task.action_config)?;
    let text = config["text"].as_str()
        .ok_or_else(|| anyhow!("resume_agent reminder has no text in action_config"))?
        .to_string();
    (text, false, false)
};
```

For callbacks: use `SilentTrigger::Callback` and call `mark_task_delivered` (existing behavior).
For reminders: use `SilentTrigger::Reminder` and **skip** `mark_task_delivered` — let `fire_task()` handle the `completed` transition.

Session prefix: `callback-{uuid}` for callbacks, `reminder-{uuid}` for reminders.

#### 3. Add `action_type` field to `create_reminder` tool

**File:** `crates/mika-agent/src/tools/create_reminder.rs`

- Add optional `action_type` field to input schema with enum `["send_message", "resume_agent"]`, default `"send_message"`
- Parse and validate the field (reject unknown values)
- Pass through to `NewTask` at line 244 (currently hardcoded)
- Update tool description to explain when to use each type:
  - `send_message`: Static notification to the user (default)
  - `resume_agent`: Wake the agent to take action (check status, run tools, etc.)

#### 4. Update `get_user_visible_tasks` query

**File:** `crates/mika-agent/src/db.rs`

Change the filter from:
```sql
WHERE (action_type = 'send_message'
    OR (trigger_type = 'callback' AND action_type = 'resume_agent'))
```

To:
```sql
WHERE (action_type IN ('send_message', 'resume_agent')
    AND trigger_type NOT IN ('callback'))
```

This ensures `list_reminders` shows `resume_agent` reminders while still excluding callback system tasks.

#### 5. Update dedup check in `create_reminder`

**File:** `crates/mika-agent/src/tools/create_reminder.rs`

The dedup check at line ~258 filters by `action_type == "send_message"`. Change to match by label regardless of `action_type`, so a user can't accidentally create duplicate reminders with different action types.

#### 6. Update `list_reminders` display

**File:** `crates/mika-agent/src/tools/list_reminders.rs`

Show a visual indicator for `resume_agent` reminders, e.g., `(action)` vs `(notification)`, so the user knows which reminders wake the agent.

#### 7. Add `reminder-` session prefix to pruning

**File:** `crates/mika-agent/src/task_engine/engine.rs` (or wherever `prune_old_sessions` is called)

Add `"reminder-"` to the list of session prefixes pruned by `startup_recovery`.

#### 8. Update system prompt guidance

**File:** `crates/mika-agent/src/prompt.rs` (or relevant prompt assembly)

Add guidance for when to use `resume_agent` vs `send_message`:
- Use `resume_agent` when the reminder requires the agent to take action (check CI, query APIs, run tools)
- Use `send_message` for static notifications ("meeting in 15 min")

## System-Wide Impact

- **Interaction graph:** `create_reminder` → task engine scheduler → `fire_task` → `dispatch_resume_agent` → `run_silent_agent` with `SilentTrigger::Reminder`. The agent may then call tools (`send_message`, `check_work_item`, etc.) within the silent turn.
- **Error propagation:** If `dispatch_resume_agent` fails for a reminder, `fire_task` handles it with the AgentBusy 30s retry or marks it failed. Same as other dispatch failures.
- **State lifecycle:** No orphan risk — reminder tasks follow the standard scheduler lifecycle (`pending → in_progress → completed`). The `dispatch_resume_agent` refactor guards `mark_task_delivered` behind `trigger_type == "callback"`.
- **API surface parity:** `list_reminders` and `cancel_reminder` both need to handle `resume_agent` reminders. Dashboard unified_timeline already shows all tasks.
- **Loop prevention:** Silent agent turns from reminders use `safe_always_on_skills()` and `default_tools()` — same safety as heartbeat and callback turns.

## Acceptance Criteria

- [x] `create_reminder` accepts optional `action_type` field (`"send_message"` default, `"resume_agent"`)
- [x] `resume_agent` reminders fire a silent agent turn with the reminder message as context
- [x] `resume_agent` reminders work in both CLI and server mode
- [x] `list_reminders` shows `resume_agent` reminders with action indicator
- [x] `cancel_reminder` works for `resume_agent` reminders
- [x] Recurring `resume_agent` reminders reschedule correctly after each fire
- [x] Dedup check catches duplicate reminders regardless of `action_type`
- [x] `SilentTrigger::Reminder` has appropriate prompt framing (not callback framing)
- [x] `reminder-` sessions are pruned by `startup_recovery`
- [x] Backward compatible — existing reminders continue to work as `send_message`
- [x] Tests cover one-shot and recurring `resume_agent` reminders
- [x] Tests cover dispatcher lifecycle for reminder vs callback paths

## Key Files

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Add `SilentTrigger::Reminder`, prompt framing |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | Refactor `dispatch_resume_agent` for dual lifecycle |
| `crates/mika-agent/src/tools/create_reminder.rs` | Add `action_type` field, update dedup |
| `crates/mika-agent/src/tools/list_reminders.rs` | Show action type indicator |
| `crates/mika-agent/src/db.rs` | Update `get_user_visible_tasks` query |
| `crates/mika-agent/src/task_engine/engine.rs` | Add `reminder-` to session pruning |
| `crates/mika-agent/src/prompt.rs` | Add guidance for `resume_agent` vs `send_message` |

## Sources

- Issue: #363 — fix: add resume_agent action type to reminders for CI follow-up
- Root cause: `dispatcher.rs:121-145` — `dispatch_send_message` silently drops when no sender
- Callback lifecycle doc: `docs/solutions/architecture/callback-resume-agent-lifecycle.md`
- Loop prevention: `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`
- CLI mode race: `docs/solutions/logic-errors/callback-processing-race-steals-tui-notifications.md`
