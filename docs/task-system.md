---
title: Task System
description: Task lifecycle reference — trigger types, action types, status transitions, and anomaly definitions
---

# Task System

Mika's task engine is a unified SQLite-backed scheduler that manages all background work — from one-shot reminders to recurring heartbeats, long-running callbacks, and user-tracked work items.

## Trigger Types

Each task has a `trigger_type` that determines how it fires:

| Trigger | Purpose |
|---|---|
| `time` | One-shot task that fires at a specific scheduled time |
| `recurring` | Repeats on a cron schedule (e.g. heartbeat hourly, reflection at 2am daily) |
| `callback` | Event-driven — created by the agent, completed by an external process (e.g. long-running exec handler) |
| `user_reply` | Fires when the user replies to the conversation |
| `event` | Fires on a system event |
| `condition` | Fires when a specific condition is met |
| `manual` | User-tracked work items created via `create_work_item` — not auto-dispatched by the task engine |
| `a2a` | Agent-to-Agent protocol tasks — created by remote A2A requests |

## Action Types

Each task has an `action_type` that determines what happens when it fires:

| Action | Purpose |
|---|---|
| `send_message` | Sends a message to the user via the configured message sender |
| `run_skill` | Executes a named skill |
| `inject_context` | Injects context into the agent's system prompt |
| `resume_agent` | Resumes the agent loop (used with callback tasks from long-running processes) |
| `invoke_orchestrator` | Fires the team orchestrator (used for team suspend/resume after delegated work completes) |
| `none` | No automated action — used by `manual` work items which are tracking-only entries |

## Status Definitions

| Status | Terminal? | Description |
|---|---|---|
| `pending` | No | Task created, waiting to fire or be acted upon |
| `recurring_active` | No | Recurring task armed and waiting for next cron fire time |
| `in_progress` | No | Task has been claimed and is currently executing |
| `blocked` | No | Work item is blocked (manual tasks only, user-managed) |
| `completed` | Yes* | Task finished successfully (*except callback tasks which proceed to `delivered`) |
| `delivered` | Yes | Callback task result has been injected into the conversation |
| `failed` | Yes | Task execution failed |
| `expired` | Yes | Task exceeded its `timeout_at` deadline |
| `cancelled` | Yes | Task was cancelled before completion |

### Terminal States

Once a task reaches any of these states, no further transitions occur:
- `completed` (except callback tasks, which proceed to `delivered`)
- `delivered`
- `failed`
- `expired`
- `cancelled`

## Status Transition Diagrams

### `time` (one-shot scheduled)

```
pending → in_progress → completed
                      → failed
         → expired    (timeout_at passed before firing)
         → cancelled  (cancelled before firing)
```

Terminal state: `completed`, `failed`, `expired`, or `cancelled`. No further work.

### `recurring` (cron-based)

```
recurring_active → in_progress → [dispatch succeeds] → recurring_active (re-armed with next cron time)
                               → failed               (cron_expr missing, dispatch error, or reschedule failure)
                 → expired     (timeout_at passed)
                 → cancelled   (explicitly cancelled)
```

Cycling state: `recurring_active` and `in_progress` alternate indefinitely on each cron tick. Only stops on `failed`, `expired`, or `cancelled`.

### `callback` (external completion)

```
pending → completed  (external process calls POST /tasks/{id}/complete or `mika ask --task-id <uuid>`)
        → delivered  (TUI/server atomically claims the completed task and injects result into conversation)
        → expired   (timeout_at passed; orphan process receives SIGTERM)
        → failed    (subprocess exits non-zero; background monitor reads capped stderr and marks failed)
```

Full lifecycle: `pending → completed → delivered`. The `completed → delivered` transition is the critical handoff where the result enters the conversation. A task stuck at `completed` without reaching `delivered` is an anomaly that requires investigation.

### `manual` (work items)

```
pending → in_progress → completed
                      → blocked    → in_progress (unblocked)
                                   → completed
                                   → cancelled
                      → cancelled
        → completed   (direct completion)
        → cancelled   (direct cancellation)
```

Validated transitions (enforced at the tool layer):
- `pending` → any status
- `in_progress` → blocked, completed, cancelled
- `blocked` → in_progress, completed, cancelled
- Terminal states (`completed`, `cancelled`) cannot transition

Manual work items are not auto-dispatched by the task engine. Status is managed via `create_work_item`, `update_work_item_status`, and `check_work_item` tools.

### `user_reply` / `event` / `condition`

```
pending → in_progress → completed
                      → failed
         → expired    (timeout_at passed)
         → cancelled
```

Same as `time`: standard one-shot lifecycle, differing only in what triggers the `pending → in_progress` transition.

## Anomaly Definitions

These are task states that indicate something may need attention:

| Anomaly | Description | What to do |
|---|---|---|
| **Stuck callback** | A callback task in `completed` status that was never transitioned to `delivered` (stuck for >10 minutes) | Investigate why the result was not delivered to the conversation. Ask the user. |
| **Failed recurring** | A recurring task (e.g. heartbeat, reflection) in `failed` status — it should be cycling but has stopped | Alert the user. The recurring schedule has broken. |
| **Long-running task** | A task `in_progress` for longer than its expected duration (>1 hour default, or 2x `estimated_duration_secs`) | Flag it to the user — the task may be stuck or the process may have died. |
| **Stale blocked item** | A manual work item in `blocked` status with no activity for >24 hours | Ask the user if the blocker has been resolved or if the item should be updated. |
| **GitHub-linked item** | A manual work item with a GitHub PR/issue `reference_url` — the linked PR may have been merged | Use `check_work_item` to inspect the PR/issue status and suggest updating the work item accordingly. |
