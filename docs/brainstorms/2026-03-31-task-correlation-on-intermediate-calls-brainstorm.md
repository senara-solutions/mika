# Task Correlation on Intermediate Calls

**Date:** 2026-03-31
**Status:** Brainstorm complete
**Scope:** mika CLI (`mika ask`), task engine, long-running skill lifecycle

## What We're Building

Add task-id correlation to **every** `mika ask` call during a long-running skill's lifetime — not just the final completion callback. This enables observability tools (traces, logs, dashboard) to link intermediate interactions (e.g., claude-pilot permission requests) back to the parent task.

Today, `mika ask --task-id <id>` means "complete this task with this result." Between task creation and that final call, intermediate interactions (permission requests, status queries) arrive as plain `mika ask --agent <name>` with **no task correlation**. They're invisible orphans in observability.

## Why This Approach

### Problem

When claude-pilot (or any long-running skill) runs, it may call back into mika multiple times before completing:
- `canUseTool` permission requests flow through the relay as `mika ask --agent mika-dev`
- These carry no reference to the spawning task
- In traces, logs, and the dashboard, these calls appear disconnected from the task that triggered them
- You can't answer "what happened during task X?" without manually correlating timestamps

### Solution: `--task-complete` flag

Every `mika ask` call during a long-running skill passes `--task-id <callback_uuid>`:

```bash
# Intermediate call (permission request, status update, etc.)
mika ask --task-id $TASK_ID --agent mika-dev -- "approve Edit on src/main.rs?"

# Final call (task completion — same semantics as today)
mika ask --task-id $TASK_ID --task-complete --agent mika-dev -- "$RESULT"
```

**Without `--task-complete`:** the task-id is recorded in session/trace metadata for correlation only. The task row in the DB is untouched. The ask is processed normally.

**With `--task-complete`:** same behavior as today's `--task-id` path — calls `update_task_completed()`, triggers callback delivery.

### Why not `--task-status` enum?

Considered a `--task-status` field with values like `in_progress`, `completed`, `failed`. Rejected because:
- The only distinction that matters at the CLI boundary is "intermediate vs. done"
- Status transitions are already managed inside the task engine — duplicating them on the CLI adds ambiguity
- A boolean flag is simpler, harder to misuse, and requires no validation of allowed values
- If we need richer status later, we can add it without breaking the boolean

### Why not `--task-context` (separate flag)?

Considered keeping `--task-id` as the completion signal and adding `--task-context` for intermediate calls. Rejected because:
- Having two different flags that both accept the same task UUID is confusing
- `--task-id` is the natural name for "which task does this relate to"
- The completion semantic was bolted onto `--task-id` by convention, not by design — `--task-complete` makes the intent explicit

## Key Decisions

1. **`--task-id` becomes correlation-only by default.** Passing `--task-id` no longer implies completion. This is a breaking change in the CLI contract.
2. **`--task-complete` is a boolean flag** that, combined with `--task-id`, triggers the existing completion path (`update_task_completed()` + callback delivery).
3. **Intermediate calls: correlate only.** When `--task-id` arrives without `--task-complete`, mika records the task-id in session/trace metadata but does not modify the task row.
4. **This applies to all long-running skills**, not just claude-pilot. The protocol is generic.
5. **The initial state is never passed via CLI.** Task creation and initial status are handled by the executor internally.

## Changes Required

### mika CLI (`ask` command)
- Add `--task-complete` boolean flag
- When `--task-id` is present without `--task-complete`: set `task_id` in session metadata / trace context, then process normally (no task state change)
- When both `--task-id` and `--task-complete` are present: existing completion path (validate callback task, `update_task_completed()`, parent reconciliation)
- Backward compatibility: callers using `--task-id` alone today expect completion. Migration needed.

### mika server (`POST /tasks/{id}/complete`)
- No change needed — this endpoint is already explicitly "complete." The CLI is the only path that overloads `--task-id`.

### claude-pilot handler (`run.sh`)
- Pass `--task-id $TASK_ID` on relay/permission `mika ask` calls (currently not passed)
- Add `--task-complete` to the final result delivery call

### claude-pilot relay transport
- Thread the task-id through `canUseTool` callback invocations so it reaches `mika ask`

### Observability
- Session metadata should include `task_id` when present, enabling dashboard queries like "show all sessions for task X"
- Trace spans should carry `task_id` as an attribute for OTel correlation

## Open Questions

None — all questions resolved during brainstorm.
