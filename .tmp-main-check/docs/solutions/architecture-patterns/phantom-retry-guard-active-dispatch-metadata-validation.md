---
module: task-engine
date: 2026-04-16
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "mika-dev sends 'Pipeline produced no commits' message 67 seconds after dispatch, before any callback returns"
  - "pipeline_retry_count metadata written to task while claude-pilot session is still running"
  - "fabricated retry triggers cascade: concurrency-guard bypass and tool filter regression"
root_cause: logic_error
resolution_type: code_fix
tags:
  - phantom-retry
  - llm-fabrication
  - metadata-guard
  - callback-lifecycle
  - defense-in-depth
  - tool-boundary-guard
related_components:
  - assistant
---

# Phantom Retry Guard: Active-Dispatch Metadata Validation

## Problem

mika-dev (the orchestrator agent) fabricated a pipeline failure 67 seconds after launching a claude-pilot session. No callback had returned yet -- the pipeline was still running. The LLM hallucinated "Pipeline produced no commits for mika#334 -- retrying (1/2)" and attempted to write `pipeline_retry_count` metadata to the task, consuming retry budget before any real failure occurred.

The existing `validate_dispatch_readiness` guard (#525) blocked the re-dispatch itself, but the fabricated metadata persisted -- polluting the retry budget so that when a real callback eventually returned, the agent believed it had already used one retry attempt.

## Symptoms

- mika-dev sends retry notifications within seconds of dispatch launch (real pipelines take minutes to hours)
- `pipeline_retry_count` metadata appears on tasks with active (pending/in_progress) callback children
- Retry budget consumed before any callback-delivered failure signal
- Cascade of secondary failures triggered by the phantom retry attempt

## What Didn't Work

- **Prompt-only enforcement**: The self-dev skill prompt already instructed mika-dev to "Wait for the completion callback" and "Do NOT proactively poll." The LLM ignored these instructions under model drift (same class of nonconformance as #308, #483, #525).
- **Relying on dispatch guard alone**: `validate_dispatch_readiness` correctly blocked re-dispatch, but the metadata write succeeded via a separate tool call (`update_task_status`) that had no corresponding guard.

## Solution

Added a code-level guard in `update_task_status` tool that rejects retry-semantic metadata writes when the task has an active callback child task:

```rust
// In update_task_status execute(), after fetching the task:
if let Some(meta) = metadata_input
    && has_retry_semantic_keys(meta)
    && let Ok(children) = ctx.db.get_child_tasks(task_id).await
{
    let active_callback = children.iter().find(|c| {
        c.trigger_type == "callback"
            && matches!(c.status.as_str(), "pending" | "in_progress")
    });
    if let Some(child) = active_callback {
        return Ok(ToolOutput::error(
            serde_json::json!({
                "error": "retry_metadata_rejected_active_dispatch",
                "task_id": task_id,
                "active_child_id": child.id,
                "active_child_status": child.status,
                "reason": "Cannot write retry-related metadata while a dispatch is still running."
            }).to_string(),
        ));
    }
}
```

The `has_retry_semantic_keys()` helper matches any top-level metadata key containing "retry" (case-insensitive), catching `pipeline_retry_count`, `qa_retry_count`, `retry_attempt`, etc.

Defense-in-depth prompt hardening was added to the self-dev skill prompt:
- Explicit prerequisite on the "On pipeline failure" section requiring a delivered callback with `PIPELINE FAILURE:` prefix
- Anti-hallucination guardrail in Step 4 ("Wait for callback")
- Calibration Rule 10 documenting the incident with Wrong/Right examples

## Why This Works

The guard operates at the tool boundary -- the only LLM-facing path for metadata writes. Engine-internal metadata writes (`try_extract_callback_metadata` in dispatcher.rs) bypass this guard since they go through `update_work_item_metadata` directly on the DB, not through the tool.

The design is fail-open: if `get_child_tasks` returns an error, the write proceeds. This is intentional -- the dispatch readiness guard (#525) is the primary defense against re-dispatch (and is fail-closed). This guard prevents the secondary harm (metadata pollution) and fail-open avoids blocking legitimate metadata writes during transient DB issues.

## Prevention

1. **Code guards at tool boundaries, not prompt instructions** -- this is the dominant pattern across 6+ prior solutions (#308, #483, #525, #531, #377). When LLM nonconformance can cause real harm, enforce in the harness.
2. **Structured JSON errors** -- LLMs reliably consume structured error responses (like `retry_metadata_rejected_active_dispatch`); they ignore plain-text advisories.
3. **Independent retry budgets with metadata persistence** -- retry counters must only be written after a real callback signal, not based on LLM inference about pipeline state.
4. **Active-child detection** -- the query pattern `get_child_tasks() + filter on trigger_type="callback" && status in (pending, in_progress)` is reusable across guards.

## Related

- #579 -- This issue
- #525 -- Dispatch readiness guard (prevents re-dispatch, primary defense)
- #308 -- Fabricated action-claim guard (same class: code guard over prompt)
- #483 -- Completion-claim guard (same class: code guard over prompt)
- `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`
- `docs/solutions/architecture-patterns/fabricated-action-claim-guard.md`
- `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md`
