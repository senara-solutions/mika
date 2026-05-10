---
title: "Deferred-dispatch wrappers fail to promote after blocking dispatch completes (autonomous loop deadlock)"
date: 2026-05-10
category: logic-errors
module: task-engine
problem_type: logic_error
component: tooling
symptoms:
  - "Autonomous loop deadlocks after first claude-pilot dispatch completes"
  - "Deferred wrappers stay pending indefinitely despite blocking callbacks reaching delivered status"
  - "pgrep -af claude-pilot returns empty — no subprocess spawns for deferred wrappers"
  - "mika-dev reports deferred dispatch fired but DB state shows wrappers still pending"
root_cause: logic_error
resolution_type: code_fix
severity: critical
tags:
  - deferred-dispatch
  - callback-promotion
  - autonomous-loop
  - deadlock
  - task-engine
  - agent-busy
---

# Deferred-dispatch wrappers fail to promote after blocking dispatch completes

## Problem

The autonomous loop deadlocks after the first claude-pilot dispatch completes. Subsequent dispatches register as deferred-dispatch wrappers (`long_running:run_claude_pilot:deferred`) but never promote or fire when the blocking dispatch completes. Observed twice on v0.12.4 with 5+ hour stuck state requiring manual `mika tasks cancel`.

## Symptoms

- Four deferred wrappers stayed `pending` despite two blocking callbacks reaching `delivered` status
- No claude-pilot subprocess spawned for any deferred wrapper
- mika-dev's own state model was wrong — it reported "the deferred iteration dispatch fired as expected" but DB state showed wrappers still pending
- Dispatch queue grew indefinitely; manual orchestrator intervention required

## What Didn't Work

- The deferred-dispatch primitive itself (PR #1061, mika#1058) worked correctly for registration — wrappers were created with the right labels and parent linkage. The failure was purely in the promotion path.
- The single inline promotion trigger at `dispatch_resume_agent` completion was insufficient — it had no backstop if it failed to fire, and its anti-cascade guard actively prevented chain promotion.

## Solution

Three independent structural fixes, each addressing one defect:

### 1. Remove anti-cascade guard (dispatcher.rs)

The guard `task.label != DEFERRED_DISPATCH_LABEL` prevented `dispatch_next_deferred_callback()` from firing when a DeferredDispatch turn itself completed. This blocked chain promotion — at most N wrappers could be promoted (one per blocking callback), with remaining wrappers stuck.

```rust
// Before: guard prevented chain promotion
if task.label != crate::agent::DEFERRED_DISPATCH_LABEL {
    self.dispatch_next_deferred_callback().await;
}

// After: always promote — each call is a LIMIT 1 DB write + return,
// not a call-stack cascade. Actual dispatch happens on next engine tick.
self.dispatch_next_deferred_callback().await;
```

### 2. Add periodic promotion backstop (engine.rs)

New `promote_pending_deferred_if_idle()` method runs every `DB_SCAN_INTERVAL_TICKS` (60 ticks). Checks `has_any_active_callback()` (excludes deferred wrappers via `label NOT LIKE '%:deferred'`) and promotes when the dispatch slot is idle. Placed BEFORE `dispatch_undelivered_callbacks` for same-tick dispatch.

```rust
async fn promote_pending_deferred_if_idle(&self) {
    match self.db.has_any_active_callback().await {
        Ok(true) => return,  // Dispatch slot occupied
        Ok(false) => {}      // Slot free — promote
        Err(e) => { warn!(...); return; }  // Fail-closed
    }
    self.dispatcher.dispatch_next_deferred_callback().await;
}
```

### 3. Fix AgentBusy callback recovery (handlers.rs)

When `dispatch_resume_agent` returned `AgentBusy`, the callback was reset from `completed` to `pending`. Neither `get_schedulable_tasks` (excludes callbacks) nor `get_undelivered_callback_tasks` (requires completed/failed) could find `pending` callbacks — they were stranded in limbo.

```rust
// Before: reset to pending — stranded the callback
db.update_task_status(&completed_task.id, task_status::PENDING).await;

// After: keep completed, just set retry delay
// dispatch_undelivered_callbacks has a next_fire_at guard that
// skips tasks whose retry delay hasn't expired
let retry_at = crate::timestamp::now_plus(chrono::Duration::seconds(30));
db.update_task_next_fire_at(&completed_task.id, &retry_at).await;
```

## Why This Works

Three defects formed a gap with no recovery path:

1. **Defect 1 (anti-cascade guard):** Each promotion is a synchronous SQLite `LIMIT 1` UPDATE that returns immediately. The promoted task dispatches on the next engine tick via `dispatch_undelivered_callbacks` — a separate async context, not the same call stack. The cascading concern in the original comment was invalid.

2. **Defect 2 (no backstop):** The sole promotion trigger was the inline call after `mark_task_delivered`. If this never fired (Defect 1, `run_silent_agent` error, AgentBusy, or any race), there was zero recovery. The periodic backstop ensures promotion eventually happens regardless of the inline path.

3. **Defect 3 (AgentBusy limbo):** The task status machine has two scan paths for callbacks — `get_schedulable_tasks` (excludes `trigger_type='callback'`) and `get_undelivered_callback_tasks` (requires `status IN ('completed', 'failed')`). A callback reset to `pending` falls through both. Keeping it `completed` with a `next_fire_at` retry delay preserves discoverability.

## Prevention

- **Backstop pattern:** Engine-level periodic scans should always exist as backstops for inline state transitions. The inline path is the fast path; the periodic scan is the recovery path. Neither should be the sole trigger for a critical state transition.
- **Status machine analysis:** When changing task status in recovery paths, trace ALL scan queries that read that status. The `pending` → `completed` → `delivered` callback lifecycle has specific scan queries at each stage — resetting to an earlier stage must be compatible with the queries that find tasks at that stage.
- **Chain promotion:** When a state machine has chained transitions (A completes → promotes B → B completes → promotes C), the promotion step must fire for ALL completions, not just the "original" ones. Anti-cascade guards on LIMIT-1 DB writes are unnecessary.

## Related Issues

- mika#1058 / PR #1061 — added the deferred-dispatch primitive (this regression is downstream)
- mika#1011 — original DeferredDispatch SilentTrigger variant
- mika#991 — PostCallbackAdvance backstop pattern (same structural approach)
- `docs/solutions/logic-errors/callback-deferred-dispatch-gate-rejection-2026-05-10.md` — predecessor fix for executor gate
- `docs/solutions/best-practices/callback-advance-2026-05-06.md` — engine-level promotion pattern
