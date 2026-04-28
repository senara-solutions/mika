---
title: "Parent self_dev task leaks in_progress when callback subtask delivers without producing artifacts"
date: 2026-04-29
category: logic-errors
module: task-engine
problem_type: logic_error
component: background_job
symptoms:
  - "Parent self_dev task stays status=in_progress indefinitely after callback subtask delivers"
  - "Operator's task list accumulates dead rows blocking future dispatches"
  - "No process or agent is touching the stuck parent task"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags:
  - task-engine
  - reaper
  - callback
  - self-dev
  - orphaned-task
  - dispatch-reliability
---

# Parent self_dev task leaks in_progress when callback subtask delivers without producing artifacts

## Problem

When a `long_running:run_claude_pilot` callback subtask is marked `delivered` but produced no PR, the parent self_dev task is left at `status=in_progress` indefinitely. Nothing reaps it. Future dispatches against the same issue collide with the stale work item, and the operator's task list accumulates dead rows.

## Symptoms

- Parent task with `source=self_dev`, `trigger_type=manual` stays `in_progress` after all callback children are in terminal states
- `SELECT id, status, source FROM tasks WHERE status='in_progress' AND source='self_dev'` returns rows older than 10+ minutes with no active process
- No PR was created despite the callback subtask transitioning to `delivered`

## What Didn't Work

- **Prompt-side guard only (#870):** The sibling fix adds a callback-turn post-condition guard requiring `update_task_status` + `send_message` before EndTurn. But that guard cannot fire if the callback turn itself crashes (LLM transport error, max-tool-steps cap, deadline exceeded) before reaching EndTurn. The engine-side reaper is needed as the safety net for this gap.

## Solution

Added `reap_orphaned_parent_tasks()` to `TaskEngine::tick()` as the 5th periodic-scan call. The reaper runs every 60-tick cycle (60s) and:

1. Queries for orphaned parents via `find_orphaned_parent_tasks()` — a SQL query that matches parents with `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'` whose callback child is `delivered`, past a 600s grace period, with no `$.claude_pilot.pr_url` in metadata, and no active sibling callbacks.

2. Transitions each match to `failed` via `update_task_failed()` (guarded UPDATE with terminal-state check) and emits an `audit_events` row with `tool_name='task_engine_reaper'`.

Atomically, extended `extract_callback_fields()` in `dispatcher.rs` to parse `pr_url` from claude-pilot output (regex `^PR:\s+<url>` matching the line emitted by `dev-pilot/handlers/run.sh:398`). Without this, the reaper's "no pr_url in metadata" check would fire on every successful run.

Key design decisions:
- **Guarded update:** Uses `update_task_failed` (not raw `update_task_status`) to avoid TOCTOU race with concurrent terminal transitions
- **GROUP BY:** SQL uses `GROUP BY parent.id` to prevent duplicate rows when a parent has multiple delivered callback children
- **NOT EXISTS sibling guard:** Defers reaping when #870's correction loop has launched a retry via `create_task`
- **600s grace period:** ~3x the upper bound of observed callback duration (187s in the mika#868 audit), long enough for #870's re-enter recovery

## Why This Works

The root cause is an architectural gap: the task engine had no reconciliation path for parents whose callback subtask lifecycle completed without updating the parent. The callback-turn prompt guard (#870) closes the loud-failure path; this engine-side reaper (#871) closes the silent-crash path where the callback turn can't fire its guards. Both are needed because they cover different failure modes at different layers.

## Prevention

- **New long-running dispatch flows should have a reaper counterpart.** If a new trigger type creates parent-child task relationships with an expected artifact (like `pr_url`), add a corresponding reaper check.
- **Use guarded updates (`update_task_failed`) for engine-side transitions** — raw `update_task_status` has no terminal-state check and can overwrite concurrent transitions.
- **Always ship extraction (R4) atomically with the reaper (R1)** — the reaper misfires without the metadata it checks, and the extraction is dead code without a consumer.

## Related Issues

- [mika#871](https://github.com/senara-solutions/mika/issues/871) — This fix
- [mika#870](https://github.com/senara-solutions/mika/issues/870) — Callback-turn assistant-message guard (sibling fix, prompt layer)
- [mika#868](https://github.com/senara-solutions/mika/issues/868) — Dev-run audit that surfaced both #870 and #871
