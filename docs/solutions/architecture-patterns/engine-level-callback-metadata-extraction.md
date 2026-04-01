---
title: "Engine-Level Callback Metadata Extraction"
category: architecture-patterns
date: 2026-04-02
tags: [callback, metadata, task-engine, dispatcher, work-item, reliability]
issue: "#376"
modules: [task_engine/dispatcher, tools/update_work_item_status]
---

# Engine-Level Callback Metadata Extraction

## Problem

When claude-pilot completes and mika-dev receives the callback, the work item's `metadata` field remained empty. No `session_id`, `cost_usd`, `duration_ms`, `turns`, `branch`, or `pr_url` was recorded. Audit commands couldn't report per-task costs, and the dashboard task detail page showed empty metadata.

**Root cause:** Metadata persistence was entirely agent-driven via prompt instructions in the self-dev skill's Step 6 (close-out). Step 6 is the **last** action in a complex callback flow that includes QA delegation, retry loops, and notifications. When the 20-step tool budget was exhausted by earlier actions, Step 6 was dropped. The `max_steps_exceeded` continuation turn runs with tools disabled, so metadata could never be recovered.

## Solution

**Combined approach: deterministic engine-level extraction + prompt restructuring.**

### Part 1: Engine-Level Extraction (Primary Fix)

Added `try_extract_callback_metadata()` in `dispatcher.rs` that runs **before** `run_silent_agent()` in `dispatch_resume_agent()`:

```rust
// In dispatch_resume_agent(), before constructing SilentAgentParams:
if is_callback {
    try_extract_callback_metadata(&self.db, task).await;
}
```

The function:
1. Checks `task.parent_task_id` references a `trigger_type='manual'` work item
2. Parses callback result text for structured fields via regex (`Session:`, `Turns:`, `Cost:`, `Duration:`)
3. Shallow-merges extracted fields into existing metadata (preserves keys like `pipeline_retry_count`)
4. Persists via `update_work_item_metadata()` — the same DB function the agent's tool uses

**Key design decisions:**
- **Fire-and-forget:** Failures logged at `warn!` but never block dispatch
- **Nested under `claude_pilot` key:** Matches the self-dev prompt's metadata schema
- **Pre-agent timing:** Guarantees base metadata even if the agent exhausts its step budget
- **`"unknown"` sentinel filtering:** Regex inherently rejects non-numeric values; session_id explicitly filters the `"unknown"` default from the handler

### Part 2: Self-Dev Prompt Restructuring (Complementary)

Moved metadata persistence from Step 6 (close-out, often dropped) to Step 3 (callback entry, runs early). The agent persists all 6 fields (including `branch` and `pr_url` which only the agent can discover) immediately after extraction, before consuming steps on QA delegation.

The shallow merge in `merge_and_persist_metadata()` means later calls enrich (never clobber) engine-written or agent-written metadata.

## Key Insight

**Prompt-only enforcement doesn't work for critical persistence.** When a workflow has N steps and the last step does the critical write, any step budget pressure drops it. The fix pattern: move the critical write to the engine (deterministic, pre-agent) for base data, and restructure the prompt to persist enrichments early (before expensive operations).

This is the same lesson as the delegation work item guard (#278): "Prompt-only enforcement is unreliable — the agent ignores instructions after compaction. Code-level guards are the only reliable enforcement mechanism."

## Prevention

- When designing agent workflows that persist data, ensure the engine writes base data deterministically before the agent runs
- Place critical tool calls early in the agent's step sequence, not at the end
- Use shallow merge patterns so engine and agent writes are complementary, not conflicting

## Files Changed

- `crates/mika-agent/src/task_engine/dispatcher.rs` — `try_extract_callback_metadata()`, `extract_callback_fields()`, 9 unit tests + 3 integration tests
- `mika-skills/self-dev/system_prompt.md` — Moved metadata persistence from Step 6 to Step 3
- `CLAUDE.md` — Documented engine-level callback metadata extraction

## Related

- `docs/solutions/architecture-patterns/generic-callback-framing-parent-task-id.md` — Threading `parent_task_id` through intermediate types
- `docs/solutions/architecture-patterns/callback-turn-work-item-context-injection.md` — Expanding trigger-type guards for callback turns
- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — Code-level enforcement over prompt instructions
