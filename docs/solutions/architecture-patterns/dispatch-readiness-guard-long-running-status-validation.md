---
title: "Dispatch-Readiness Guard: Status Validation Before Long-Running Dispatch"
date: 2026-04-13
category: architecture-patterns
module: mika-agent (skills/executor.rs)
problem_type: logic_error
component: tooling
symptoms:
  - "Redundant claude-pilot session dispatched on task already in in_progress with open PR"
  - "Fabricated UUID passed to run_claude_pilot returned soft error but did not prevent retry with real ID"
  - "LLM ignored create_task advisory about existing task status"
root_cause: missing_validation
resolution_type: code_fix
severity: high
tags:
  - dispatch-guard
  - work-item-status
  - long-running
  - double-dispatch
  - code-guard
  - defense-in-depth
  - executor
related_issues:
  - "#525"
  - "#522"
  - "#524"
---

# Dispatch-Readiness Guard: Status Validation Before Long-Running Dispatch

## Problem

`execute_long_running()` in `skills/executor.rs` dispatched long-running subprocess (claude-pilot) sessions without validating whether the task was in a dispatchable state. On 2026-04-11, mika-dev dispatched a redundant claude-pilot session on a task that was already `in_progress` with PR #522 open, QA-approved, and CI green. The redundant session burned ~$0.18 and contributed to a 7-hour desync where the PR sat unmerged.

## Symptoms

- `run_claude_pilot` accepted a fabricated UUID, returned "Task not found", and the LLM retried with the correct task_id
- `create_task` returned a soft advisory "Task already exists... Status: in_pr" but the LLM ignored it
- A second claude-pilot session ran against a task that already had an active callback child task

## What Didn't Work

- **Prompt-level enforcement**: The system prompt instructed the agent not to re-dispatch tasks in terminal states, but the LLM ignored this during recovery from a misclassified webhook
- **Soft advisory from `create_task`**: The dedup return string included the status, but the LLM treated it as informational rather than blocking
- **Existing `validate_task()` check**: Already in place but too permissive — accepts `blocked` status (needed for `delegate_task`) and does not check for active child tasks

## Solution

Added `validate_dispatch_readiness()` as a stricter second-pass validation in `execute_long_running()`, after the existing `validate_task()` call. The new guard enforces two checks:

1. **Status check**: Only `pending` and `in_progress` are dispatchable. `blocked`, `completed`, and `cancelled` are rejected with structured JSON error (`task_not_dispatchable`).
2. **Active-child check**: Queries `get_child_tasks(task_id)` and filters for `trigger_type == "callback" && status IN (pending, in_progress)`. If any active callback child exists, dispatch is rejected with `task_active_dispatch` error.

Additionally, `pending` tasks are auto-transitioned to `in_progress` before creating the callback task, closing the TOCTOU window for sequential double-dispatch.

Key design decisions:
- **Separate from `validate_task()`**: The shared helper is co-consumed by `delegate_task` which intentionally allows `blocked`. Modifying it would change delegation behavior.
- **Structured JSON errors**: Error responses include `error`, `task_id`, `current_status`, `pr_url`, and `reason` fields for programmatic LLM feedback.
- **Fail-closed on DB errors**: If `get_child_tasks()` fails, dispatch is rejected rather than silently proceeding.
- **`Result<String, String>` return type**: Returns the task status on success, eliminating a redundant DB read in the auto-transition logic.

## Why This Works

The root cause was that dispatch-readiness was delegated entirely to LLM judgment. The LLM can hallucinate UUIDs, ignore advisory strings, and improvise during recovery from misclassified webhooks. The tool boundary is the only enforcement point that cannot be talked around.

The guard follows the established pattern from the completion-claim guard (#483) and delegation work-item guard: "If the agent ignoring an instruction would cause real harm, enforce it in the harness."

## Prevention

- **Code guards over prompt instructions** for any action with real resource cost (subprocess spawn, API calls, state mutations). Prompt-level enforcement is defense-in-depth, not primary.
- **Active-child detection** prevents double-dispatch even when the task status looks correct (`in_progress` is valid but may already have an active session).
- **Auto-transition on dispatch** ensures `pending` items cannot be double-dispatched via the TOCTOU window between status check and callback creation.
- **Structured JSON errors** give the LLM programmatic feedback to adjust behavior, rather than relying on string parsing of plain-text errors.

## Related Issues

- [#525](https://github.com/senara-solutions/mika/issues/525) — This issue
- [#522](https://github.com/senara-solutions/mika/pull/522) — The stuck PR incident that triggered this work
- [#524](https://github.com/senara-solutions/mika/issues/524) — Companion: structural verdict handler (webhook side)
- [Delegation task guard](delegation-work-item-guard-enforcement.md) — The original code-guard pattern this builds on
- [Completion-claim guard](completion-claim-guard-work-item-state-enforcement.md) — Post-condition guard pattern for fabricated claims
- [Callback task loop prevention](callback-task-loop-prevention.md) — Related: prevents callback turns from spawning new long-running tasks
