# Plan: Fix Deferred-Dispatch Promotion Deadlock (mika#1070)

type: bug
issue: mika#1070
branch: fix/1070/engine-deferred-dispatch-wrappers-fail

## Problem

Deferred-dispatch wrappers (registered when `global_dispatch_active` fires) never promote after the blocking dispatch completes. The autonomous loop stalls indefinitely — observed twice on v0.12.4 (post-PR #1061 + #1065) with 5+ hour stuck state requiring manual `mika tasks cancel`.

### Empirical evidence

Four deferred wrappers (`398d327c`, `6e304d9c`, `061d43f6`, `3df441b2`) stayed `pending` despite two blocking callbacks (`630e4ac3`, `79fa162f`) reaching `delivered` status. `pgrep -af claude-pilot` returned empty — no subprocess ever spawned for any deferred wrapper.

## Pinned Source (Phase 0)

### Pin 1: Anti-cascade guard (dispatcher.rs:449-474)

Full context of `dispatch_resume_agent` post-turn handling:

```rust
// dispatcher.rs:449-474 — inside dispatch_resume_agent(), after run_silent_agent completes
        if let Err(e) = run_silent_agent(&params).await {
            warn!(task_id = %task.id, error = %e, "resume_agent run failed");
        } else if is_callback {
            // Mark delivered so TUI polling doesn't re-process this callback.
            // Only for callbacks — reminder lifecycle is managed by fire_task().
            if let Err(e) = self.db.mark_task_delivered(&task.id).await {
                warn!(task_id = %task.id, error = %e, "failed to mark callback task as delivered");
            }

            // #991 — Post-callback advance backstop.
            self.maybe_fire_post_callback_advance(task).await;

            // mika#1011 — Drain next pending deferred-dispatch callback (FIFO).
            // The blocking dispatch just completed, so the slot is free. Promote
            // the oldest pending deferred callback and dispatch it immediately.
            // This must run AFTER mark_task_delivered to ensure the blocking
            // callback is fully processed before the next dispatch fires.
            if task.label != crate::agent::DEFERRED_DISPATCH_LABEL {
                // Only drain from non-deferred callback completions to prevent
                // cascading deferred dispatches from draining the whole queue
                // in a single call stack (each deferred turn drains one more).
                self.dispatch_next_deferred_callback().await;
            }
        }
```

### Pin 2: dispatch_next_deferred_callback (dispatcher.rs:954-968)

```rust
// dispatcher.rs:954-968
    async fn dispatch_next_deferred_callback(&self) {
        match self.db.promote_next_deferred_callback().await {
            Ok(true) => {
                info!("deferred_dispatch_promoted — task marked completed for engine dispatch");
            }
            Ok(false) => {} // No pending deferred callbacks
            Err(e) => {
                warn!(error = %e, "failed to promote deferred callback — will retry on next tick");
            }
        }
    }
```

Key: `dispatch_next_deferred_callback()` performs a **DB state transition only** (UPDATE tasks SET status='completed'). It does NOT inline-dispatch. The actual dispatch happens later when `dispatch_undelivered_callbacks()` scans for `status IN ('completed', 'failed')` on the next engine tick cycle. The function returns immediately after the DB write.

### Pin 3: promote_next_deferred_callback SQL (db.rs:5337-5359)

```rust
// db.rs:5337-5359
    pub fn promote_next_deferred_callback(&self, agent_id: &str) -> Result<bool> {
        let now = crate::timestamp::now();
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'completed',
                 result = 'deferred dispatch slot freed',
                 completed_at = ?3,
                 next_fire_at = ?3,
                 updated_at = ?3
             WHERE id = (
                 SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'pending'
                   AND label = 'long_running:run_claude_pilot:deferred'
                 ORDER BY created_at ASC
                 LIMIT 1
             )
             AND agent_id = ?2",
            params![agent_id, agent_id, now],
        )?;
        Ok(n > 0)
    }
```

This is a synchronous SQLite `execute()` that commits immediately (autocommit). The write is visible to all subsequent reads on any connection (WAL mode). Promotes exactly ONE wrapper (LIMIT 1), oldest first (ORDER BY created_at ASC).

### Pin 4: AgentBusy recovery (handlers.rs:457-491)

```rust
// handlers.rs:457-491 — inside handle_task_complete spawned tokio task
            match dispatcher.dispatch_resume_agent(&completed_task).await {
                Err(crate::task_engine::DispatchError::AgentBusy(_)) => {
                    let now = crate::timestamp::now();
                    let is_expired = completed_task
                        .timeout_at
                        .as_ref()
                        .is_some_and(|ts| ts.as_str() <= now.as_str());

                    if is_expired {
                        warn!(task_id = %completed_task.id, "task timed out while waiting for agent, marking failed");
                        if let Err(db_err) = db
                            .update_task_failed(
                                &completed_task.id,
                                "task timed out while waiting for agent",
                            )
                            .await
                        { ... }
                    } else {
                        // Agent is busy — reset task to pending for the tick loop to retry
                        debug!(task_id = %completed_task.id, "agent busy, deferring resume_agent to tick loop in 30s");
                        let retry_at = crate::timestamp::now_plus(chrono::Duration::seconds(30));
                        if let Err(e) = db
                            .update_task_status(&completed_task.id, task_status::PENDING)
                            .await
                        { ... }
                        if let Err(e) = db
                            .update_task_next_fire_at(&completed_task.id, &retry_at)
                            .await
                        { ... }
                    }
                }
```

Note: `update_task_completed` (line 436) runs BEFORE this match block, setting status to `completed`. The `AgentBusy` branch then resets it to `pending`. The spawned tokio task is fire-and-forget from the HTTP handler's perspective — the HTTP response (200 OK) was already sent.

### Pin 5: scan_db_for_new_tasks query (db.rs:4343-4356)

```rust
// db.rs:4343-4356
    pub fn get_schedulable_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND status IN ('pending','recurring_active')
               AND trigger_type NOT IN ('callback', 'manual')
             ORDER BY next_fire_at ASC NULLS LAST",
            Self::TASK_COLUMNS
        );
        // ...
    }
```

Explicitly excludes `callback` and `manual` trigger types. A callback task reset to `pending` by the AgentBusy path will NOT be found by this scan.

### Pin 6: dispatch_undelivered_callbacks query (db.rs:4977-4994)

```rust
// db.rs:4977-4994
    pub fn get_undelivered_callback_tasks(&self, agent_id: &str, since: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('completed', 'failed')
               AND completed_at IS NOT NULL
               AND completed_at > ?2
             ORDER BY completed_at ASC",
            Self::TASK_COLUMNS
        );
        // ...
    }
```

Requires `status IN ('completed', 'failed')`. A callback task reset to `pending` by the AgentBusy path will NOT be found by this scan either.

## Root Cause Analysis

Three structural defects in the promotion path, each independently sufficient to cause the deadlock. Together they form a gap with no recovery path.

### Empirical trace through defects

The ticket shows two blocking callbacks delivered and four deferred wrappers stuck:
- `630e4ac3` delivered → at Pin 1 line 469, `task.label = "long_running:run_claude_pilot"` (non-deferred) → condition TRUE → `dispatch_next_deferred_callback()` called → should promote `061d43f6` (oldest wrapper, created 17:38:03). **If this path fires correctly**, `061d43f6` transitions to `completed` and the engine scan dispatches it as a DeferredDispatch turn. During that turn, `run_claude_pilot` may hit `global_dispatch_active` (if `79fa162f` is still active), registering yet another deferred wrapper. Either way, when the DeferredDispatch turn completes → `mark_task_delivered` on `061d43f6` → **Defect 1 fires**: line 469 condition is FALSE (deferred label) → no chain promotion → remaining wrappers stuck.
- `79fa162f` delivered → same path → promotes `3df441b2` → same Defect 1 chain-break.

**Defect 1 alone** explains why at most 2 of 4 wrappers could ever be promoted (one per blocking callback completion). **Defect 2** (no backstop) explains why there's no recovery. **Defect 3** (AgentBusy limbo) is a latent amplifier — if either callback hit AgentBusy before being dispatched, even the first promotion would never fire.

### Defect 1: Anti-cascade guard prevents chain promotion (dispatcher.rs:469)

At Pin 1 line 469, the guard `task.label != DEFERRED_DISPATCH_LABEL` prevents `dispatch_next_deferred_callback()` from running when a deferred dispatch turn completes.

**Promotion-to-dispatch execution trace (proving async safety for guard removal):**

1. `dispatch_next_deferred_callback()` is called inside `dispatch_resume_agent()` (Pin 2)
2. It calls `promote_next_deferred_callback()` (Pin 3) — a synchronous SQLite UPDATE that sets `status = 'completed'` and commits immediately (autocommit)
3. The function returns. `dispatch_resume_agent()` returns. The agent lock is released.
4. On the next engine tick cycle (up to 60 seconds later), `dispatch_undelivered_callbacks()` (engine.rs:230) runs
5. It calls `get_undelivered_callback_tasks()` (Pin 6) which finds the promoted task (`status = 'completed'`)
6. It spawns a NEW `tokio::spawn` (engine.rs:387) that calls `dispatch_resume_agent()` for the promoted task
7. That NEW `dispatch_resume_agent()` acquires the agent lock via `try_lock()` (dispatcher.rs:371)
8. If agent is busy → `DispatchError::AgentBusy` → error silently dropped → next scan retries
9. If agent is free → DeferredDispatch silent turn runs → promotion at line 469 fires for the next wrapper

**Conclusion:** Steps 1-3 are a DB write + return. Steps 4-9 happen in a separate async context on a subsequent tick. There is no call-stack cascade. Each `dispatch_next_deferred_callback()` call writes one DB row and returns. The concern in the original comment ("each deferred turn drains one more") describes the desired chain behavior, not a problem. Removing the guard is safe.

### Defect 2: No engine-level periodic promotion backstop

The sole promotion trigger is the inline call at Pin 1 line 473. If this never fires (Defect 1, `run_silent_agent` error at Pin 1 line 449, AgentBusy, or any race), there's no recovery. The engine has `dispatch_undelivered_callbacks` (scans for `completed`/`failed`) and `scan_db_for_new_tasks` (excludes callbacks), but neither can promote `pending` deferred wrappers.

### Defect 3: `handle_task_complete` AgentBusy recovery strands callbacks (latent)

At Pin 4, when `dispatch_resume_agent` returns `AgentBusy`, the callback is reset from `completed` to `pending` (line 481). Per Pin 5 and Pin 6, neither scan path finds `pending` callbacks:
- `get_schedulable_tasks` → `trigger_type NOT IN ('callback', 'manual')` → excluded
- `get_undelivered_callback_tasks` → `status IN ('completed', 'failed')` → excluded

The callback is in limbo. If this is the BLOCKING callback, `dispatch_resume_agent` never runs for it, so the promotion at line 473 never fires.

## Changes

### Change 1: Remove anti-cascade guard, allow chain promotion (dispatcher.rs)

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`
**Lines:** 464-474

Replace:
```rust
if task.label != crate::agent::DEFERRED_DISPATCH_LABEL {
    self.dispatch_next_deferred_callback().await;
}
```

With:
```rust
self.dispatch_next_deferred_callback().await;
```

Remove the guard entirely. The cascading concern is invalid — each promoted wrapper fires as a separate `dispatch_resume_agent` invocation on the next engine scan, not in the same call stack. The promotion SQL (db.rs:5337) promotes ONE wrapper (LIMIT 1), so each callback completion promotes at most one wrapper. Chain promotions happen across separate turns, not within one.

### Change 2: Add periodic deferred-dispatch promotion scan (engine.rs)

**File:** `crates/mika-agent/src/task_engine/engine.rs`

Add a new method `promote_pending_deferred_if_idle()` called at `DB_SCAN_INTERVAL_TICKS` cadence (same block as `dispatch_undelivered_callbacks`, reap, etc.):

```rust
/// Engine-level backstop for deferred-dispatch promotion (mika#1070).
///
/// Runs every DB_SCAN_INTERVAL_TICKS. If pending deferred wrappers exist
/// AND no active callback child exists for any task, promotes the oldest
/// wrapper. This recovers from any scenario where the inline promotion
/// at dispatcher.rs:473 fails to fire.
async fn promote_pending_deferred_if_idle(&self) {
    // Check if any callback is currently in-flight
    match self.db.has_any_active_callback().await {
        Ok(true) => return, // Dispatch slot occupied — don't promote
        Ok(false) => {}     // Slot free — check for promotable wrappers
        Err(e) => {
            warn!(error = %e, "failed to check active callbacks for deferred promotion");
            return; // Fail-closed
        }
    }
    // Promote the oldest pending deferred wrapper
    self.dispatcher.dispatch_next_deferred_callback().await;
}
```

Add `has_any_active_callback` query to `db.rs`:

```rust
/// Returns true if any callback task is in pending/in_progress status
/// (i.e., a dispatch slot is occupied). Used by the engine-level
/// deferred-dispatch backstop (mika#1070).
pub fn has_any_active_callback(&self, agent_id: &str) -> Result<bool> {
    let count: i64 = self.conn.query_row(
        "SELECT COUNT(*) FROM tasks
         WHERE agent_id = ?1
           AND trigger_type = 'callback'
           AND action_type = 'resume_agent'
           AND status IN ('pending', 'in_progress')
           AND label NOT LIKE '%:deferred'",
        params![agent_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}
```

The `label NOT LIKE '%:deferred'` exclusion prevents deferred wrappers (in any status — pending, completed/promoted, or in_progress/dispatching) from blocking the idle check. Deferred wrappers are managed by their own promotion path, not the regular callback pipeline. Regular callbacks (`long_running:run_claude_pilot`, `long_running:run_shell`, etc.) are the only ones that occupy the dispatch slot.

Wire into the engine tick at `DB_SCAN_INTERVAL_TICKS`, **BEFORE** `dispatch_undelivered_callbacks`:
```rust
if self.tick_count.is_multiple_of(DB_SCAN_INTERVAL_TICKS) {
    // ... existing scans ...
    if !self.dispatcher.cli_mode {
        self.promote_pending_deferred_if_idle().await;  // NEW — promote first
        self.dispatch_undelivered_callbacks().await;     // then dispatch
    }
    // ...
}
```

**Promote-first ordering:** `promote_next_deferred_callback` (Pin 3) is a synchronous SQLite UPDATE that commits before returning (autocommit). The write is visible to all subsequent reads. Placing promotion BEFORE `dispatch_undelivered_callbacks` means a promoted wrapper can be dispatched in the SAME tick cycle (60s improvement over promote-after). No race: the promotion function returns only after the DB write commits.

### Change 3: Fix AgentBusy callback recovery (handlers.rs)

**File:** `crates/mika-agent/src/server/handlers.rs`
**Lines:** 476-491

Replace the `AgentBusy` branch:
```rust
Err(DispatchError::AgentBusy(_)) => {
    let now = crate::timestamp::now();
    let is_expired = completed_task
        .timeout_at
        .as_ref()
        .is_some_and(|ts| ts.as_str() <= now.as_str());

    if is_expired {
        warn!(task_id = %completed_task.id, "task timed out while waiting for agent, marking failed");
        if let Err(db_err) = db
            .update_task_failed(&completed_task.id, "task timed out while waiting for agent")
            .await
        {
            warn!(...);
        }
    } else {
        // Keep status as 'completed' (already set at line 436) so
        // dispatch_undelivered_callbacks can find it on the next scan.
        // Only update next_fire_at for the retry delay.
        debug!(task_id = %completed_task.id, "agent busy, deferring resume_agent to callback scan in ~60s");
        let retry_at = crate::timestamp::now_plus(chrono::Duration::seconds(30));
        if let Err(e) = db
            .update_task_next_fire_at(&completed_task.id, &retry_at)
            .await
        {
            warn!(...);
        }
    }
}
```

Then add a `next_fire_at` guard in `dispatch_undelivered_callbacks` (engine.rs:365, inside the `for task in tasks` loop) to skip tasks whose `next_fire_at` is in the future:

```rust
for task in tasks {
    // Retry delay guard: skip tasks whose next_fire_at is in the future
    // (AgentBusy retry delay, mika#1070)
    let now = crate::timestamp::now();
    if let Some(ref fire_at) = task.next_fire_at {
        if fire_at.as_str() > now.as_str() {
            continue;
        }
    }
    // ... existing staleness guard and dispatch ...
}
```

#### Double-dispatch race analysis (F3 resolution)

**Race A:** HTTP handler spawns task → `update_task_completed` (status=completed) → `dispatch_resume_agent` → AgentBusy → keeps completed + sets `next_fire_at`. Meanwhile, engine tick fires `dispatch_undelivered_callbacks` and finds the same completed task.

**Resolution:** The `next_fire_at` guard in `dispatch_undelivered_callbacks` prevents premature dispatch. The `update_task_next_fire_at` write is a synchronous SQLite UPDATE (autocommit, single writer) that completes before the HTTP handler's `AgentBusy` branch returns. The engine tick runs on a 1-second interval; the next `dispatch_undelivered_callbacks` scan runs at `DB_SCAN_INTERVAL_TICKS` (60 ticks = 60 seconds). The 30-second retry delay is shorter than the 60-second scan interval, so by the time the scan fires, `next_fire_at` will be in the past and the task will be dispatched.

The actual concurrency protection against double-dispatch is the **agent lock** (`try_lock()` at dispatcher.rs:371). `dispatch_resume_agent` acquires the agent lock — if two concurrent calls attempt to process the same task, only one acquires the lock; the other returns `AgentBusy`. After the winning call processes the task and calls `mark_task_delivered` (status → `delivered`), subsequent scans skip it (Pin 6 queries `status IN ('completed', 'failed')` — `delivered` is excluded).

**Race B (overlapping retry windows):** Cannot occur. The HTTP handler's spawned task is fire-and-forget — it does NOT retry after `AgentBusy`. Only `dispatch_undelivered_callbacks` retries, and it runs at 60-second intervals (far longer than the 30-second retry delay). There is no window for two retry attempts to overlap.

**Additional defense-in-depth:** `mark_task_delivered` is idempotent — calling it twice on the same task is a no-op (UPDATE WHERE status != 'delivered'). Even if two `dispatch_resume_agent` calls somehow ran the same callback turn concurrently (extremely unlikely given the agent lock), the second `mark_task_delivered` would be harmless.

### Change 4: Add `deferred_dispatch_promoted` INFO log (dispatcher.rs)

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`
**Lines:** 956-957

Enhance the existing promotion log with structured fields per AC:

```rust
Ok(true) => {
    info!(
        event = "deferred_dispatch_promoted",
        "promoted oldest pending deferred wrapper for engine dispatch"
    );
}
```

### Change 5: Regression test (eval or db test)

**File:** `crates/mika-agent/src/task_engine/engine.rs` (new test module) or `crates/mika-agent/src/db.rs`

Add a test that exercises the full chain:
1. Create a parent task P
2. Create a regular callback child C for P (simulates blocking dispatch)
3. Register deferred wrapper W via `register_deferred_callback`-like DB insertion
4. Mark C as `completed` (simulates claude-pilot completion)
5. Call `promote_next_deferred_callback` (simulates the promotion at line 473)
6. Assert W transitions to `completed` and has `completed_at` set
7. Assert `get_undelivered_callback_tasks` returns W
8. Register a second deferred wrapper W2
9. Mark W as `delivered` (simulates DeferredDispatch turn completion)
10. Call `promote_next_deferred_callback` again (chain promotion, Change 1)
11. Assert W2 transitions to `completed`

For the engine backstop (Change 2), add a test:
1. Create a deferred wrapper W with no active callbacks
2. Call `promote_pending_deferred_if_idle`
3. Assert W is promoted

For the AgentBusy fix (Change 3), add a test:
1. Create a callback task, mark completed
2. Simulate AgentBusy (keep status as completed, set next_fire_at)
3. Assert `get_undelivered_callback_tasks` returns the task
4. Assert the task is NOT returned before `next_fire_at` (retry delay guard)

## Acceptance Criteria Mapping

| AC | Change |
|----|--------|
| Deferred wrappers promoted within one scheduler tick | Changes 1 + 2 |
| Regression test | Change 5 |
| `deferred_dispatch_promoted` INFO log event | Change 4 |
| DeferredDispatch silent trigger fires | Already working (PR #1061 injected `LongRunningContext`) |

## Risk Assessment

- **Change 1** (remove anti-cascade guard): Low risk. The concern about cascading was about call-stack depth, but promotions happen across separate async invocations. Each promotion is LIMIT 1.
- **Change 2** (periodic backstop): Low risk. Additive — doesn't change existing paths. Fail-closed on DB errors. Only promotes when dispatch slot is genuinely idle.
- **Change 3** (AgentBusy fix): Medium risk. Changes the recovery behavior for AgentBusy. The key invariant is that the task must stay `completed` so the scan picks it up. Need to verify that `dispatch_undelivered_callbacks` handles re-dispatching completed tasks correctly (it does — it spawns `dispatch_resume_agent` which acquires the agent lock and processes the task).
- **Change 5** (tests): No risk.

## Out of Scope

- The per-turn dispatch counter (`dispatch_count: AtomicU32`) limiting one dispatch per turn — this is correct behavior and not related to the deadlock.
- The flood cap (MAX_PENDING_DEFERRED_CALLBACKS = 10) — correct safety limit.
- The `global_dispatch_active` guard itself — correct single-session-at-a-time invariant.
- The `DeferredDispatch` `LongRunningContext` injection — already fixed by PR #1061 (mika#1058).
