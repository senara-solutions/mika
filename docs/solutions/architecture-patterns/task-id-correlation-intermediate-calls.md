---
title: Task-ID correlation for intermediate long-running skill calls
category: architecture-patterns
date: 2026-04-01
tags: [cli, task-engine, observability, tracing, session-metadata, breaking-change]
module: mika-cli, mika-agent
issue: 358
---

# Task-ID Correlation for Intermediate Long-Running Skill Calls

## Problem

When a long-running skill (e.g., claude-pilot) runs, it may issue multiple intermediate `mika ask` calls (permission requests, status queries) before the final completion callback. Previously, `--task-id` on `mika ask` meant "complete this task" — so intermediate calls could not carry the task ID without triggering premature completion. This created an observability blind spot: intermediate interactions were invisible orphans in traces and the dashboard, with no way to answer "what happened during task X?" without manual timestamp correlation.

## Root Cause

The `--task-id` CLI flag overloaded two distinct concepts: "which task this relates to" (correlation) and "this task is done" (completion). A single flag cannot serve both purposes when intermediate interactions need correlation without completion.

## Solution

Split `--task-id` into correlation-only (default) and completion (explicit `--task-complete` flag):

```bash
# Intermediate call — correlate only, run full agent loop
mika ask --task-id $TASK_ID --agent mika-dev -- "approve Edit on src/main.rs?"

# Final call — complete the task
mika ask --task-id $TASK_ID --task-complete --agent mika-dev -- "$RESULT"
```

### Key implementation details

1. **CLI flag**: `--task-complete` added with `requires("task_id")` in clap. Without it, `--task-id` is correlation-only.

2. **Session metadata**: Task ID stored in session metadata JSON via `serde_json::json!({"task_id": tid})` (not `format!()` — prevents JSON injection). Dashboard queries via `json_extract(metadata, '$.task_id')`.

3. **AgentParams threading**: `correlated_task_id: Option<String>` added to `AgentParams`. Recorded on the `agent_turn` tracing span using `tracing::field::Empty` default with conditional `span.record()` — avoids polluting traces with empty attributes on the ~99% of calls that have no task correlation.

4. **Input validation**: `--task-id` validated for non-empty and max 128 chars before any DB operations.

5. **Deprecation bridge**: When `--task-id` targets a completable callback without `--task-complete`, a stderr warning and `tracing::warn!` are emitted. This protects against silent breakage during the cross-repo transition (mika-skills handlers must be updated to add `--task-complete`).

6. **100KB size limit**: Moved from the `task_id.is_some()` gate to the `task_complete` gate. Intermediate calls (which may carry large code diffs) are no longer size-limited.

7. **Dashboard API**: `GET /api/v1/sessions` accepts optional `task_id` query parameter. No schema migration — uses existing `sessions.metadata` JSON column.

### Pattern: `Option<String>` field propagation

This follows the established `parent_task_id` and `trace_id` propagation pattern:
- Add field to `AgentParams` as `Option<String>`
- All call sites default to `None` (server, A2A, chat, eval harness)
- CLI sets `Some(value)` when the user passes `--task-id`
- **Audit**: After implementation, grep for `correlated_task_id: None` — any site where task context is available but passes `None` is a propagation bug

### Not propagated to `SilentAgentParams` or `TeamAgentParams`

Intentional asymmetry. Silent/team paths have their own task correlation via `SilentTrigger::Callback` carrying `parent_task_id`. The `correlated_task_id` field is for conversation-mode intermediate calls only.

## Prevention

- When adding CLI flags that carry identifiers, separate "correlation" from "action" semantics from the start. A single flag should not mean "relate to X" and "act on X" depending on context.
- Use `serde_json::json!()` for constructing metadata JSON — never `format!()` with user-supplied values.
- Use `tracing::field::Empty` for optional span attributes to avoid noise in telemetry pipelines.
- When making breaking CLI changes, add a deprecation bridge if cross-repo coordination is required. Include the version/timeframe for removal in the warning message.

## Related

- [docs/solutions/architecture-patterns/generic-callback-framing-parent-task-id.md](generic-callback-framing-parent-task-id.md) — Same `Option<String>` propagation pattern for `parent_task_id`
- [docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md](trace-id-structural-linkage-delegate-silent-callback.md) — `trace_id` threading pattern
- [docs/solutions/architecture-patterns/cli-flag-id-suffix-convention.md](cli-flag-id-suffix-convention.md) — `--{noun}-id` naming convention
- [docs/brainstorms/2026-03-31-task-correlation-on-intermediate-calls-brainstorm.md](../../brainstorms/2026-03-31-task-correlation-on-intermediate-calls-brainstorm.md) — Original brainstorm
- GitHub issue: #358
