---
module: skills-executor
date: 2026-05-10
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "claude-pilot session exits Success after `mika ask --agent mika-arch` returns — verdict discarded, 0 commits"
  - "executor.rs gate rejects run_claude_pilot from callback turns with 'long-running tool invoked without long_running_ctx'"
  - "DeferredDispatch silent turns fail at INTENT_GUARD because run_claude_pilot is blocked by the same gate"
  - "Pipeline retry path on PIPELINE FAILURE callbacks cannot invoke run_claude_pilot"
root_cause: logic_error
resolution_type: code_fix
tags:
  - callback
  - deferred-dispatch
  - long-running
  - executor-gate
  - cycle-detection
  - pipeline-retry
---

# Callback and DeferredDispatch Turns Cannot Call Long-Running Tools

## Problem

Two failure modes prevented pipeline retry and deferred dispatch from working:

1. **Callback gate rejection (Mode B):** When mika-dev's pipeline-retry path fires on a `PIPELINE FAILURE` callback, the retry attempts `run_claude_pilot` directly. This fails at `executor.rs:278-296` because callback turns have `long_running_ctx = None`. The gate is an `Option` check — not behavioral — and cannot be overcome by prompt changes.

2. **DeferredDispatch latent bug:** The existing `DeferredDispatch` mechanism (mika#1011) passes `None` for `long_running_ctx` to `run_loop` for ALL silent triggers (line 3339: `None, // long_running not supported in silent mode`). But the `deferred_dispatch_action` INTENT_GUARD requires the LLM to call `run_claude_pilot`. The LLM tries, gets blocked by the gate, correction fires, blocked again → max steps exceeded.

## What Didn't Work

- **Prompt-only enforcement:** LLMs rationalize crossing prompt budgets; the gate is a structural `Option` check, not behavioral.
- **Watchdog re-spawn from dispatch-lib:** Parallel retry mechanism in bash, requires verdict-extraction-from-DB coupling, becomes tech debt the moment the engine fix lands.

## Solution

Two-part fix:

### Part 1: Inject `LongRunningContext` for DeferredDispatch triggers

In `run_silent_inner`, construct a `LongRunningContext` conditionally for `SilentTrigger::DeferredDispatch` and pass it to `run_loop` instead of unconditional `None`:

```rust
let long_running_ctx =
    if matches!(&params.trigger, SilentTrigger::DeferredDispatch { .. }) {
        Some(executor::LongRunningContext {
            db: db.clone(),
            agent_name: db.agent_id().to_string(),
            session_id: params.session_id.to_string(),
            trace_id: trace_id.clone(),
            dispatch_count: AtomicU32::new(0),
        })
    } else {
        None
    };
```

### Part 2: Gate-intercept for callback and DeferredDispatch turns

Added `callback_task_id: Option<&str>` to `ToolContext`, threaded from `SilentTrigger::Callback` and `SilentTrigger::DeferredDispatch`. When the executor gate rejects a long-running tool call and `callback_task_id` is `Some`:

1. Run `check_lineage_cycle()` — walks `parent_task_id` chain (max 4 hops), compares `(repo, issue_number, skill)` tuples
2. If no cycle: call `register_deferred_callback()` to enqueue the dispatch
3. Return `{"status": "deferred", "deferred": true}` — LLM knows not to retry

```rust
if let Some(task_id) = callback_task_id
    && let Some(db) = callback_db
{
    match check_lineage_cycle(db, task_id, &input).await {
        Ok(()) => {
            if register_deferred_callback(db, task_id, &input).await {
                return ToolOutput::success(json!({"status": "deferred", ...}));
            }
        }
        Err(cycle_msg) => {
            return ToolOutput::error(json!({"error": "deferred_dispatch_cycle_detected", ...}));
        }
    }
}
```

### Cycle detection design

Lineage walk on `(repo, issue_number, skill)` tuple using existing `parent_task_id` chain:
- ✅ Allows `groom-#159 → pilot-#159` (different skill)
- ✅ Catches `groom-#159 → retry-groom-#159` (same tuple)
- ✅ Catches A→B→A class via lineage walk
- Fail-open on extraction failure; `depth ≤ 3` schema CHECK is structural backstop

## Why This Works

The gate at `executor.rs` was intentional defensive code (commit `04ae084c`) to prevent loop-like behavior in callback turns. The fix preserves the gate for non-deferred contexts (heartbeat, reflection, CLI test) while enabling the specific pattern the engine already supports: deferred dispatch through `register_deferred_callback()`.

DeferredDispatch turns are the engine's auto-recovery path for `global_dispatch_active` rejections — their sole purpose is to call `run_claude_pilot`. Blocking them from doing so via the gate was a latent bug from the original silent-mode `None` assignment.

## Prevention

- When adding new `SilentTrigger` variants that need to execute long-running tools, construct `LongRunningContext` explicitly rather than inheriting the blanket `None` for silent mode
- The `callback_task_id` field on `ToolContext` is the signal — if a turn can legitimately defer a long-running dispatch, it must carry the task ID for cycle detection
- Test new dispatch paths with the executor gate: verify both the `long_running_ctx = Some` path (gate passes) and the `callback_task_id` intercept path (gate catches and defers)
