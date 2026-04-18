---
title: Terminal-State Metadata Rejection Race Between Verdict Handler and Callback
date: 2026-04-18
category: logic-errors
module: mika-agent/tools/update_task_status
problem_type: logic_error
component: tooling
symptoms:
  - "update_task_status rejects metadata writes on completed/cancelled tasks with 'terminal state' error"
  - "Late-arriving callback metadata (cost_usd, duration_ms, pr_url, session_id) silently lost after verdict_handler auto-completes a task"
  - "Task metadata incomplete — merge metadata present but claude_pilot metadata missing"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags:
  - terminal-state
  - metadata
  - race-condition
  - update-work-item-status
  - callback
  - verdict-handler
---

# Terminal-State Metadata Rejection Race Between Verdict Handler and Callback

## Problem

`update_task_status` rejected the entire call when the status transition was invalid, even if the caller only wanted to add metadata. Terminal-state tasks (`completed`, `cancelled`) could never receive metadata updates from late-arriving callbacks, causing data loss.

## Symptoms

- `verdict_handler` (structural Rust handler) completes a task in ~1 second on QA pass + CI green
- ~60 seconds later, mika-dev's callback turn tries to persist `claude_pilot` metadata (`cost_usd`, `duration_ms`, `pr_url`, `session_id`) with `status: "in_progress"`
- Tool returns error: *"Cannot transition from 'completed' to 'in_progress'. 'completed' is a terminal state."*
- Task metadata after the race is incomplete — merge metadata from verdict_handler is present, but claude_pilot callback metadata is lost

## What Didn't Work

- The tool coupled status validation with metadata writes — there was no way to write metadata without also passing a valid status transition. The rejection happened before the metadata merge was reached.

## Solution

Added a **terminal-state metadata fallback** in the transition validation block of `update_task_status`. When the transition is invalid AND the current status is terminal AND metadata is provided, the tool applies the metadata and returns success without changing the status.

Key code change in `crates/mika-agent/src/tools/update_task_status.rs`:

```rust
// Validate the transition against the state machine
if !is_valid_transition(&old_status, status) {
    let allowed = allowed_transitions(&old_status);

    // Terminal-state metadata fallback (#617): when the task is in a terminal
    // state and the caller provided metadata, apply the metadata and skip the
    // status change instead of rejecting the entire call.
    if allowed.is_empty() {
        if let Some(new_meta) = metadata_input {
            merge_and_persist_metadata(task_id, new_meta, ctx).await?;
            return Ok(ToolOutput::success(format!(
                "Status unchanged ('{old_status}' is terminal). Metadata updated."
            )));
        }
        // No metadata provided — reject as before
        return Ok(ToolOutput::error(format!(
            "Cannot transition from '{old_status}' to '{status}'. \
             '{old_status}' is a terminal state — ..."
        )));
    }

    // Non-terminal invalid transitions still fully rejected
    return Ok(ToolOutput::error(format!(
        "Cannot transition from '{old_status}' to '{status}'. \
         Valid transitions from '{old_status}': {}.",
        allowed.join(", ")
    )));
}
```

Also updated:
- Tool description to document that metadata can be written to terminal-state tasks
- System prompt at `crates/mika-agent/src/prompt.rs` to say "status locked, metadata still writable" instead of "cannot be changed"

## Why This Works

The root cause was the tool coupling status validation with metadata writes. The fix decouples them for the specific case of terminal states: when the task is already in a final state and the caller provides metadata, the status transition is silently skipped and only the metadata write executes. This preserves the terminal-state guard (no status changes allowed) while allowing metadata enrichment from late-arriving callbacks.

The fix is narrowly scoped:
- Only fires when `allowed_transitions()` returns an empty slice (true only for `completed` and `cancelled`)
- Non-terminal invalid transitions (e.g., `in_progress → pending`) are still fully rejected even with metadata
- Status-only calls without metadata on terminal tasks are still rejected
- The phantom retry guard (#579) runs before this path, so retry-semantic metadata is still blocked on active dispatches

## Prevention

- **When adding validation guards that reject writes, consider whether the rejection should be total or partial.** In this case, the status validation was correct, but it accidentally blocked the metadata write that rode alongside it.
- **Race conditions between structural handlers and LLM-driven callbacks are inherent to the architecture.** The structural handler (verdict_handler) is fast and Rust-native; the callback handler is LLM-driven and slow. Design tools to handle late-arriving writes gracefully rather than assuming callers will always have fresh state.
- **Update all prompt surfaces when changing tool behavior.** The review caught that the system prompt still said "cannot be changed" after the tool was updated — this would have caused the LLM to avoid attempting the metadata write.

## Related Issues

- [#617](https://github.com/senara-solutions/mika/issues/617) — this fix
- [#608](https://github.com/senara-solutions/mika/issues/608) — task vocabulary refactor
- [#609](https://github.com/senara-solutions/mika/issues/609) — milestone callback routing
- `docs/solutions/architecture-patterns/work-item-status-transition-validation.md` — transition state machine
- `docs/solutions/architecture-patterns/phantom-retry-guard-active-dispatch-metadata-validation.md` — phantom retry guard
- `docs/solutions/architecture-patterns/work-item-metadata-two-level-shallow-merge.md` — metadata merge semantics
