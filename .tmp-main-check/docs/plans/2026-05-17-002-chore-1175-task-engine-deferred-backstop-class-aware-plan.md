---
title: "chore(task-engine): make periodic deferred-dispatch backstop class-aware (mika#1175)"
ticket: mika#1175
type: chore
labels: [bug, p2-normal, agent-core]
branch: chore/1175/task-engine-make-periodic-deferred
related:
  - mika#1163 (PR #1170) — slot-guard `:deferred` exclusion (predecessor; merged 2026-05-17)
  - mika#1070 — periodic-backstop introduction
  - mika#1001 — per-class dispatch slot split (the feature this unmasks)
  - mika#1011 — deferred-dispatch auto-recovery design
---

# chore(task-engine): make periodic deferred-dispatch backstop class-aware (mika#1175)

## TL;DR

`promote_pending_deferred_if_idle()` (the 60-second periodic backstop that promotes deferred wrappers when the dispatch slot is idle) currently picks the **single oldest** wrapper agent-wide, irrespective of `dispatch_class`. With one `implement` wrapper and one `groom` wrapper co-pending — each occupying its own class slot — only one promotes per 60s tick. The other waits a full additional cycle, halving deferred throughput under cross-class load. Fix: make the backstop class-aware. Iterate over `dispatch_class` values, gate per-class on a class-scoped active-callback predicate, promote at most one wrapper per idle class per tick.

## Context

mika#1011 introduced deferred-dispatch auto-recovery: when `run_claude_pilot` is rejected by the per-class slot guard (`global_dispatch_active`), `register_deferred_callback` enqueues a `pending` callback task with label `long_running:run_claude_pilot:deferred` and `dispatch_class` matching the source dispatch (executor.rs:1487, executor.rs:1556). When the blocking dispatch completes, the deferred wrapper is promoted to `completed` and the engine's periodic scan picks it up on the next tick, firing a `SilentTrigger::DeferredDispatch` turn whose only required action is to re-call `run_claude_pilot`.

mika#1070 introduced two promotion paths:

1. **Inline** — `dispatch_next_deferred_callback()` runs from `handle_task_complete` after a non-deferred callback delivers (`dispatcher.rs:990`). Anti-cascade guard at `dispatcher.rs:495` (added by mika#1124) skips inline chain-promotion when the completing task is itself a `:deferred` wrapper.
2. **Periodic backstop** — `promote_pending_deferred_if_idle()` runs every `DB_SCAN_INTERVAL_TICKS` (60 ticks ≈ 60 seconds) from the engine tick loop (`engine.rs:234`, `engine.rs:429`). Calls `has_any_active_callback()` to check the dispatch slot; if idle, calls `dispatcher.dispatch_next_deferred_callback()` which promotes the **oldest** pending wrapper agent-wide.

mika#1163 (PR #1170, merged 2026-05-17) closed an asymmetric-exclusion bug: the per-class slot predicate `has_active_callback_tasks_excluding()` was counting `:deferred` wrappers as active dispatches, causing a deadlock when two parents each held a pending wrapper. The fix added `AND label NOT LIKE '%:deferred'` to the SQL predicate (`db.rs:5866`), bringing it into parity with the sibling `has_any_active_callback()` (`db.rs:5947`) which already had the exclusion. The slot-guard path is now correct.

The remaining gap is in the **promotion path** itself: `has_any_active_callback()` is agent-wide (not class-scoped), and `promote_next_deferred_callback()` (`db.rs:5913`) is also agent-wide — it ORDERs by `created_at ASC LIMIT 1` across all classes. The backstop's class-blindness is a no-op when only one class is in use; it becomes a throughput bug when both `implement` and `groom` classes have co-pending wrappers.

## What's broken

**Failure shape:**

- Defer N≥2 wrappers across different dispatch classes (e.g., 1 with `dispatch_class='implement'` + 1 with `dispatch_class='groom'`) at the same agent.
- Free both class slots (no active non-deferred callbacks anywhere).
- The 60s backstop tick fires `promote_pending_deferred_if_idle()`. `has_any_active_callback()` returns `false` (correct — both slots are idle). The backstop calls `dispatch_next_deferred_callback()` which calls `promote_next_deferred_callback()`. The DB promotes the **single oldest** wrapper (one class). The OTHER class wrapper stays `pending` until the next 60s tick (when the just-promoted wrapper's dispatch is either still in-flight or has just freed). Throughput across classes is serialized when it should be parallel.

**Why this matters now:** mika#1001 carved out a second concurrency slot (`groom` runs in parallel with `implement`). The backstop's agent-wide ordering negates that parallelism whenever wrappers across both classes are pending. Pre-#1001 (single global slot) this had no observable effect.

## Why now / dependency surface

- **Unblocks:** mika#1001 effective concurrency under cross-class deferral. Without the fix, two cross-class wrappers experience a 60s second-promotion delay — visible in any workflow that bursts a grooming + implementation dispatch on the same agent (the autonomous-loop sprint cadence).
- **Sibling of mika#1163:** Both fix asymmetric class-handling in the deferred path. mika#1163 fixed the *slot predicate*; this fixes the *promotion path*. After both ship, every entry point that touches deferred wrappers respects `dispatch_class`.
- **No downstream tickets blocked:** Cosmetic throughput fix from the caller's perspective — wrappers eventually drain. Quality-of-service bug, not a correctness bug.

## Acceptance Criteria

- **R1.** With 1 `implement` + 1 `groom` deferred wrapper pending and both class slots idle, both wrappers promote in the same 60s tick. (AC from issue body.)
- **R2.** With 2 `implement` wrappers pending and the implement slot idle (no other implement callback active), exactly **one** wrapper promotes per tick — preserves the existing single-class FIFO semantics.
- **R3.** With 1 `implement` + 1 `groom` wrapper pending and the implement slot **occupied** (active non-deferred implement callback in flight) + groom slot idle, only the groom wrapper promotes. (Gate per-class, don't block the idle class on the busy class.)
- **R4.** With 1 `implement` + 1 `groom` wrapper pending and both class slots occupied, no wrapper promotes.
- **R5.** Pre-v34 NULL `dispatch_class` rows are treated as `'implement'` via `COALESCE` (matches existing convention in `db.rs:5865`). The class iteration must include `'implement'` so NULL-class wrappers continue to promote.
- **R6.** No regression in single-class scenarios (mika#1011 design), in the inline-promotion path (mika#1070), in mika#1124's anti-cascade guard, or in mika#1163's symmetric `:deferred` exclusion.

## Out of scope / Non-goals

- **Inline-promotion path unchanged.** `dispatch_next_deferred_callback()` in `handle_task_complete` (`dispatcher.rs:990`) remains agent-wide. See `Open question 1` below for the reasoning; the ticket explicitly scopes to the periodic backstop. If the architect deems the inline path the same bug, fold it in; otherwise file a follow-up.
- **No unification of the two slot predicates.** `has_any_active_callback()` (agent-wide) and `has_active_callback_tasks_excluding()` (per-class, excluding-self) stay separate — same rationale as mika#1163 plan (refactor risks bundling regressions into a quality-of-service fix).
- **No new dispatch_class values.** The iteration stays scoped to the two currently-defined classes (`'implement'`, `'groom'`). Adding a third class is a different ticket.
- **60s cadence unchanged.** Same cadence as mika#1070 (`DB_SCAN_INTERVAL_TICKS`).
- **No saturation guard.** The per-tick promotion budget is one per class (two total today). If a future class explosion makes single-tick promotion unwieldy, file a follow-up to add a global per-tick cap.

## Phase 0 — Verbatim baselines

**Base SHA:** `ec9b8858d0aa33e080926e1be24eaacbcbc842ba` (origin/main as of 2026-05-17, includes #1163/#1170 + #1162/#1174).

Implementer reference: these are the verbatim current-state code slices the implementation will modify. If the worktree base ever drifts from `ec9b8858`, regenerate the plan rather than implementing against drift. (Pin per mika-arch first-pass F1.)

### `engine.rs:423-441` — the backstop fn under change

```rust
/// Engine-level backstop for deferred-dispatch promotion (mika#1070).
///
/// Runs every `DB_SCAN_INTERVAL_TICKS`. If pending deferred wrappers exist
/// AND no active non-deferred callback exists for the agent, promotes the
/// oldest wrapper. This recovers from any scenario where the inline
/// promotion at `dispatch_resume_agent` (dispatcher.rs) fails to fire.
async fn promote_pending_deferred_if_idle(&self) {
    // Check if any non-deferred callback is currently in-flight
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

### `db.rs:5907-5935` — agent-wide promotion (kept; sibling added)

```rust
/// Promote the next pending deferred-dispatch callback for dispatch (FIFO).
///
/// Sets `next_fire_at` to now and marks with a synthetic result so the task
/// engine's periodic scan picks it up and routes through `dispatch_resume_agent`
/// within one tick (~1 second). Returns `true` if a task was promoted.
/// Called by the dispatcher after a blocking callback completes (mika#1011).
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

### `db.rs:5937-5952` — agent-wide active-callback check (kept; sibling added)

```rust
/// Returns true if any non-deferred callback task is in pending or in_progress
/// status (i.e., a dispatch slot is occupied). Used by the engine-level
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

### `db.rs:5852-5875` — per-class slot predicate (reference; do-not-modify, sibling shape to mirror)

```rust
pub fn has_active_callback_tasks_excluding(
    &self,
    excluded_parent_id: &str,
    agent_id: &str,
    dispatch_class: &str,
) -> Result<Option<(String, String)>> {
    let mut stmt = self.conn.prepare(
        "SELECT parent_task_id, id FROM tasks
         WHERE trigger_type = 'callback'
           AND status IN ('pending', 'in_progress')
           AND parent_task_id IS NOT NULL
           AND parent_task_id != ?1
           AND agent_id = ?2
           AND COALESCE(dispatch_class, 'implement') = ?3
           AND label NOT LIKE '%:deferred'
         LIMIT 1",
    )?;
    // …
}
```

The two new methods (Unit 1) copy this exact `COALESCE(dispatch_class, 'implement') = ?` clause and the `label NOT LIKE '%:deferred'` exclusion.

### `dispatcher.rs:982-1003` — inline-path dispatcher entry (kept agent-wide; sibling added)

```rust
/// mika#1011 — Promote the next pending deferred-dispatch callback to `completed`.
///
/// Called after a blocking callback completes (mark_task_delivered succeeded),
/// and by the engine-level periodic backstop (mika#1070).
/// Promotes the oldest pending deferred callback to `completed` (FIFO).
/// The task engine's periodic scan picks up `completed` resume_agent tasks
/// and dispatches them via `dispatch_resume_agent`, which constructs a
/// `SilentTrigger::DeferredDispatch` turn for tasks with the deferred label.
pub(crate) async fn dispatch_next_deferred_callback(&self) {
    match self.db.promote_next_deferred_callback().await {
        Ok(true) => {
            info!(
                event = "deferred_dispatch_promoted",
                "promoted oldest pending deferred wrapper for engine dispatch"
            );
        }
        Ok(false) => {} // No pending deferred callbacks
        Err(e) => {
            warn!(error = %e, "failed to promote deferred callback — will retry on next tick");
        }
    }
}
```

### `executor.rs:760-768` — class-derivation function (reference; defines the universe)

```rust
fn derive_dispatch_class(skill: Option<&str>) -> &'static str {
    match skill {
        Some("dev-groom") => "groom",
        _ => "implement", // dev-pilot, deploy_mika, and all others
    }
}
```

The `DISPATCH_CLASSES: &[&str] = &["implement", "groom"]` slice in `engine.rs` (Unit 4) MUST stay in sync with this function. Test 5 (Unit 6) enforces this at test-time.

## Files and Anchors

Primary change surfaces:

- **`crates/mika-agent/src/db.rs:5913`** — `promote_next_deferred_callback()`. Add class-scoped sibling `promote_next_deferred_callback_for_class()`.
- **`crates/mika-agent/src/db.rs:5940`** — `has_any_active_callback()`. Add class-scoped sibling `has_any_active_callback_for_class()`.
- **`crates/mika-agent/src/async_db.rs:623`** — `promote_next_deferred_callback()` async wrapper. Add `promote_next_deferred_callback_for_class()` wrapper.
- **`crates/mika-agent/src/async_db.rs:630`** — `has_any_active_callback()` async wrapper. Add `has_any_active_callback_for_class()` wrapper.
- **`crates/mika-agent/src/task_engine/dispatcher.rs:990`** — `dispatch_next_deferred_callback()`. Add class-scoped sibling `dispatch_next_deferred_callback_for_class()`. Keep the agent-wide version for the inline path.
- **`crates/mika-agent/src/task_engine/engine.rs:429`** — `promote_pending_deferred_if_idle()`. Change body from "if-idle, promote one" to "for each class, if class-idle, promote one of that class".

Reference (do-not-modify) anchors:

- **`crates/mika-agent/src/db.rs:5852`** — `has_active_callback_tasks_excluding()` (gate-side per-class predicate from mika#1163). Already class-aware; provides the SQL shape we mirror.
- **`crates/mika-agent/src/skills/executor.rs:763`** — `derive_dispatch_class()`. Defines the two-class universe (`"groom"` for `dev-groom`, `"implement"` otherwise).
- **`crates/mika-agent/src/skills/executor.rs:1487`** — `register_deferred_callback()`. Wrapper task creation; `dispatch_class` is already set per #1001 (executor.rs:1556).
- **`crates/mika-agent/src/task_engine/dispatcher.rs:495`** — mika#1124 anti-cascade guard. Untouched. Inline promotion is the only path it gates.

## Approach

**Hybrid: add class-scoped DB primitives + iterate at the engine layer.** Two design alternatives were considered:

- **A. Single iterator at DB layer.** A new `promote_pending_deferred_per_class()` that returns the count of promoted wrappers. Pro: one trip to the DB. Con: hides the per-class slot check from the engine; the slot check (`has_any_active_callback_for_class`) and the promotion (`promote_next_deferred_callback_for_class`) belong to different consistency concerns, and braiding them inside a single DB call obscures both.
- **B. Engine-layer iteration over class-scoped primitives** (chosen). The engine iterates over a `&'static [&'static str]` slice of class names; for each class it independently runs the slot check, then the promotion. Pro: mirrors the existing engine-side check-then-promote pattern (`engine.rs:431-440`), keeps each DB primitive single-purpose. Con: two DB round-trips per class. At two classes and a 60s cadence, this is 4 round-trips per minute — negligible against the existing per-tick load.

Class list is `const DISPATCH_CLASSES: &[&str] = &["implement", "groom"];` co-located with the engine fn. Hardcoded against `derive_dispatch_class`'s two-arm enum at `executor.rs:763`. A `Test 5` shape-test pins drift detection (see Test Plan).

**Atomicity is per-class, not cross-class.** The engine loops are sequential. Between the slot check for `implement` and the slot check for `groom`, a new `implement` dispatch could land. This is fine — the worst case is "promoted a groom wrapper while a fresh implement dispatch raced in," which is identical to the existing single-class behavior and self-correcting (the promoted groom wrapper's class slot is independent of the implement slot).

**Race against `register_deferred_callback`** (engine fires its slot check, executor.rs registers a fresh wrapper in the same class) is also fine: a fresh wrapper is `pending`, not `in_progress` non-deferred, so it doesn't count as slot-occupying (per mika#1163's exclusion clause that we mirror). The promoted wrapper will eventually dispatch (or be re-deferred if the new wrapper somehow consumed the slot in between — bounded retry handled by the existing 60s backstop).

## Implementation Plan

### Unit 1: Add class-scoped DB primitives in `db.rs`

**Goal:** Add two class-scoped sibling methods next to the existing agent-wide ones. SQL mirrors the existing predicates with a single added `COALESCE(dispatch_class, 'implement') = ?` clause.

**Changes:**

1. After `promote_next_deferred_callback` (at `db.rs:5935`), add:

```rust
/// Class-scoped sibling of `promote_next_deferred_callback`. Promotes the
/// oldest pending deferred wrapper matching the given `dispatch_class`.
/// Returns `true` if a task was promoted. Used by the periodic backstop's
/// per-class iteration (mika#1175). Pre-v34 NULL rows treated as 'implement'
/// via COALESCE (matches `has_active_callback_tasks_excluding` semantics).
pub fn promote_next_deferred_callback_for_class(
    &self,
    agent_id: &str,
    dispatch_class: &str,
) -> Result<bool> {
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
               AND COALESCE(dispatch_class, 'implement') = ?4
             ORDER BY created_at ASC
             LIMIT 1
         )
         AND agent_id = ?2",
        params![agent_id, agent_id, now, dispatch_class],
    )?;
    Ok(n > 0)
}
```

2. After `has_any_active_callback` (at `db.rs:5952`), add:

```rust
/// Class-scoped sibling of `has_any_active_callback`. Returns `true` if any
/// non-deferred callback task in the given `dispatch_class` is `pending` or
/// `in_progress` (i.e., the per-class dispatch slot is occupied). Used by
/// the periodic backstop's per-class slot check (mika#1175). Excludes
/// `:deferred` wrappers (parity with mika#1163's symmetric exclusion).
/// Pre-v34 NULL rows treated as 'implement' via COALESCE.
pub fn has_any_active_callback_for_class(
    &self,
    agent_id: &str,
    dispatch_class: &str,
) -> Result<bool> {
    let count: i64 = self.conn.query_row(
        "SELECT COUNT(*) FROM tasks
         WHERE agent_id = ?1
           AND trigger_type = 'callback'
           AND action_type = 'resume_agent'
           AND status IN ('pending', 'in_progress')
           AND label NOT LIKE '%:deferred'
           AND COALESCE(dispatch_class, 'implement') = ?2",
        params![agent_id, dispatch_class],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}
```

**Verification:** `cargo build -p mika-agent` succeeds. New methods are reachable.

**Files modified:** `crates/mika-agent/src/db.rs` (add ~50 lines, two methods).

### Unit 2: Async-wrapper plumbing in `async_db.rs`

**Goal:** Expose both class-scoped methods through `AsyncDatabase` (the public-facing wrapper) so the dispatcher and engine can call them. Mirrors the existing wrapper pattern at `async_db.rs:623` / `:630`.

**Changes:**

Add after `promote_next_deferred_callback` and `has_any_active_callback` wrappers (around `async_db.rs:633`):

```rust
/// Class-scoped sibling of `promote_next_deferred_callback` (mika#1175).
pub async fn promote_next_deferred_callback_for_class(
    &self,
    dispatch_class: &str,
) -> Result<bool> {
    let a = self.agent_id.clone();
    let c = dispatch_class.to_string();
    self.with_db(move |db| db.promote_next_deferred_callback_for_class(&a, &c))
        .await
}

/// Class-scoped sibling of `has_any_active_callback` (mika#1175).
pub async fn has_any_active_callback_for_class(
    &self,
    dispatch_class: &str,
) -> Result<bool> {
    let a = self.agent_id.clone();
    let c = dispatch_class.to_string();
    self.with_db(move |db| db.has_any_active_callback_for_class(&a, &c))
        .await
}
```

**Verification:** `cargo build -p mika-agent` succeeds.

**Files modified:** `crates/mika-agent/src/async_db.rs` (add ~20 lines).

### Unit 3: Class-scoped dispatcher entry point

**Goal:** Add a class-scoped sibling of `dispatch_next_deferred_callback()` for the backstop to call. Keep the existing agent-wide method for the inline path (out-of-scope by design).

**Changes:**

After `dispatch_next_deferred_callback` (at `dispatcher.rs:990`), add:

```rust
/// mika#1175 — Class-scoped sibling of `dispatch_next_deferred_callback`.
/// Promotes the oldest pending deferred wrapper matching the given
/// `dispatch_class`. Used by the periodic backstop's per-class iteration.
pub(crate) async fn dispatch_next_deferred_callback_for_class(
    &self,
    dispatch_class: &str,
) {
    match self
        .db
        .promote_next_deferred_callback_for_class(dispatch_class)
        .await
    {
        Ok(true) => {
            info!(
                event = "deferred_dispatch_promoted",
                dispatch_class,
                "promoted oldest pending deferred wrapper for engine dispatch"
            );
        }
        Ok(false) => {} // No pending deferred callbacks in this class
        Err(e) => {
            warn!(
                error = %e,
                dispatch_class,
                "failed to promote deferred callback — will retry on next tick"
            );
        }
    }
}
```

**Note on log shape:** The `event = "deferred_dispatch_promoted"` field stays the same so existing grep patterns work; the new `dispatch_class` field is additive.

**Verification:** `cargo build -p mika-agent` succeeds.

**Files modified:** `crates/mika-agent/src/task_engine/dispatcher.rs` (add ~25 lines).

### Unit 4: Per-class iteration in the engine backstop

**Goal:** Change the body of `promote_pending_deferred_if_idle()` from agent-wide check-then-promote to per-class loop.

**Changes:**

Replace the body of `promote_pending_deferred_if_idle()` at `engine.rs:429`:

```rust
/// Engine-level backstop for deferred-dispatch promotion (mika#1070, mika#1175).
///
/// Runs every `DB_SCAN_INTERVAL_TICKS`. For each `dispatch_class`, if pending
/// deferred wrappers of that class exist AND no active non-deferred callback
/// exists in that class, promotes the oldest wrapper of that class. Per-class
/// iteration (mika#1175) prevents cross-class throughput halving when wrappers
/// from multiple classes are pending. This recovers from any scenario where
/// the inline promotion at `dispatch_resume_agent` (dispatcher.rs) fails to fire.
async fn promote_pending_deferred_if_idle(&self) {
    // The two currently-defined dispatch classes (per `derive_dispatch_class`
    // at executor.rs:763). Iteration order is `implement` first because pre-v34
    // NULL-class wrappers fall into this bucket via COALESCE — promote those
    // before grooming wrappers when both classes are idle. (Ordering is cosmetic
    // — both classes process independently in a single tick.)
    const DISPATCH_CLASSES: &[&str] = &["implement", "groom"];

    for class in DISPATCH_CLASSES {
        match self.db.has_any_active_callback_for_class(class).await {
            Ok(true) => continue, // Class slot occupied — skip this class
            Ok(false) => {}       // Class slot free — try to promote one wrapper
            Err(e) => {
                warn!(
                    error = %e,
                    dispatch_class = class,
                    "failed to check active callbacks for deferred promotion"
                );
                continue; // Fail-closed for this class — try the others
            }
        }
        self.dispatcher
            .dispatch_next_deferred_callback_for_class(class)
            .await;
    }
}
```

**Why `continue` not `return` on per-class error:** A DB error checking the `implement` slot should not stall the `groom` slot's promotion (independent concerns). Each class fails-closed in isolation.

**Why `const` slice not enum:** The two-class universe is already encoded as string literals throughout the codebase (`derive_dispatch_class`, `has_active_callback_tasks_excluding` callsites, `register_deferred_callback`). Promoting to an enum is a separate refactor; introducing a parallel typed representation here would invite drift. The shape-test in Unit 6 pins the universe.

**Verification:** `cargo build -p mika-agent` succeeds. `cargo clippy -p mika-agent` clean.

**Files modified:** `crates/mika-agent/src/task_engine/engine.rs` (replace ~12 lines with ~25 lines).

### Unit 5: DB-layer tests in `db.rs`

**Goal:** Pin the new DB primitives' class-filter semantics with sibling-shape tests to the existing `test_promote_next_deferred_callback_fifo` (`db.rs:16257`) and `test_has_any_active_callback` (`db.rs:16388`).

**Changes:**

Add two `#[test]` functions in the existing `mod tests` block, placed adjacent to their non-class-scoped siblings:

**Test 1: `test_promote_next_deferred_callback_for_class_filters_by_class`**

- Seed: 1 deferred wrapper with `dispatch_class='implement'`, 1 with `dispatch_class='groom'`, 1 with `dispatch_class=NULL`.
- Assert: `promote_next_deferred_callback_for_class(agent, "groom")` returns `true` and only the groom wrapper transitions to `completed`.
- Assert: `promote_next_deferred_callback_for_class(agent, "implement")` returns `true` and one of (the implement wrapper, the NULL wrapper) transitions — older-created-at wins per FIFO.
- Assert: after both calls, all three wrappers are in terminal state.

**Test 2: `test_has_any_active_callback_for_class_class_scoped`**

- Seed: 1 active non-deferred callback with `dispatch_class='implement'`.
- Assert: `has_any_active_callback_for_class(agent, "implement")` returns `true`.
- Assert: `has_any_active_callback_for_class(agent, "groom")` returns `false`.
- Assert: adding a `:deferred` wrapper in either class does NOT flip the predicate (mirrors mika#1163's exclusion).
- Assert: NULL-class active callback is matched by `"implement"` predicate (COALESCE check).

**Verification:** `cargo test -p mika-agent --lib promote_next_deferred_callback_for_class` and the `has_any_active_callback_for_class` test pass.

**Files modified:** `crates/mika-agent/src/db.rs` (add ~80 lines of tests).

### Unit 6: Engine-side regression test

**Goal:** Pin the per-class iteration end-to-end. Demonstrates that with cross-class wrappers pending and both class slots idle, both promote in the same backstop tick (R1).

**Changes:**

Add to `crates/mika-agent/src/task_engine/engine.rs` test module (search for existing engine tests; place near `engine.rs` tests if a `mod tests` block exists, or in `task_engine/dispatcher.rs`'s test module if engine has no test fixtures — verify during Unit 1 work).

**Test 3: `test_promote_pending_deferred_if_idle_iterates_per_class`**

- Build an engine fixture with two parent tasks, register one `:deferred` wrapper per class (one `implement`, one `groom`), no active non-deferred callbacks.
- Call `promote_pending_deferred_if_idle()` once.
- Assert: both wrappers transitioned from `pending` to `completed` in a single tick.

**Test 4: `test_promote_pending_deferred_if_idle_skips_busy_class`**

- Build engine fixture. Register one `:deferred` wrapper of each class. Insert an active non-deferred callback in the `implement` class only.
- Call `promote_pending_deferred_if_idle()`.
- Assert: groom wrapper transitioned to `completed`; implement wrapper still `pending`.

**Test 5: `test_dispatch_classes_universe_matches_derive_fn`**

Drift detector for the `const DISPATCH_CLASSES` shape. Calls `derive_dispatch_class(Some("dev-groom"))`, `derive_dispatch_class(Some("dev-pilot"))`, `derive_dispatch_class(None)`, asserts each result is in the `DISPATCH_CLASSES` slice. If a new class is added to `derive_dispatch_class` (e.g., a third skill maps to a new class), this test will surface the gap.

**Verification:** `cargo test -p mika-agent --lib promote_pending_deferred_if_idle` passes; `cargo test -p mika-agent` overall green.

**Files modified:** Test module in `crates/mika-agent/src/task_engine/engine.rs` (or `dispatcher.rs` if more idiomatic — confirm during implementation).

### Unit 7: CLAUDE.md update

**Goal:** Update the "Promotion paths (mika#1070)" bullet in `crates/mika-agent/CLAUDE.md` to reflect per-class iteration.

**Changes:**

Find the bullet starting with `**Promotion paths (mika#1070):**` (search for `mika#1070`). Update the periodic-backstop description to:

> (2) Periodic backstop — `promote_pending_deferred_if_idle()` runs every `DB_SCAN_INTERVAL_TICKS` (60 ticks), iterates over the two `dispatch_class` values (`'implement'`, `'groom'`), checks `has_any_active_callback_for_class(class)` (excludes deferred wrappers via `label NOT LIKE '%:deferred'`), and promotes one wrapper per idle class per tick (mika#1175). Per-class iteration prevents cross-class throughput halving when wrappers from multiple classes are co-pending. Placed BEFORE `dispatch_undelivered_callbacks` for same-tick dispatch. Fail-closed per-class on DB errors.

Update the parenthetical mention of `has_any_active_callback` so callers grepping for the slot predicate find both forms.

**Verification:** Re-grep `crates/mika-agent/CLAUDE.md` for `mika#1070` and `has_any_active_callback` — the per-class iteration claim and the new method name should be locatable.

**Files modified:** `crates/mika-agent/CLAUDE.md` (1 bullet, ~5 line delta).

## Risks

1. **Per-class iteration order matters when both classes pre-v34-NULL.** The class iteration order is `implement` first. Pre-v34 NULL-class wrappers `COALESCE` to `'implement'`, so they promote on the `'implement'` pass. If iteration order ever changes (e.g., alphabetical, or "groom" first), pre-v34 NULL wrappers still promote correctly because they only match the `'implement'` predicate — order is cosmetic. Documented in the inline comment on `DISPATCH_CLASSES`.
2. **New class addition silently halves throughput.** If `derive_dispatch_class` gains a third class but `DISPATCH_CLASSES` doesn't, the new class's deferred wrappers stop promoting. Mitigation: `Test 5` (shape-drift detector). Compile-time detection would require enum-ifying `dispatch_class`, which is out of scope per Non-goals.
3. **Same-tick race with `register_deferred_callback`.** Between slot check and promotion in a single class loop iteration, a new wrapper could register. Worst case: the slot check sees idle, we promote, the executor races in with a new dispatch attempt that registers as `in_progress` non-deferred. The promoted wrapper hits the per-class gate and re-defers via mika#1011's path. Bounded retry handled by the next 60s tick. Same race semantics as the pre-fix behavior — no new risk.
4. **Saturation:** Two classes × one promotion per tick = 2 promotions per minute. Acceptable for the current class universe. If a 4+-class explosion happens later, file a follow-up to add a per-tick global cap.

## Open questions / Surfaces for the architect

These are deliberate scope deferrals — not implementation uncertainties. Each is well-formed enough to either fold in or file as a follow-up.

### Open question 1: Should the inline-promotion path also become class-aware?

The inline path (`dispatch_next_deferred_callback` called from `handle_task_complete`, `dispatcher.rs:990`) currently promotes the **oldest** wrapper across all classes when a non-deferred callback completes.

**Failure shape (if not fixed):** Imagine 1 implement + 1 groom dispatch both running; 1 implement + 1 groom wrapper both pending. The implement dispatch completes → inline promotion picks the oldest wrapper agent-wide. If the groom wrapper is older, we promote groom — but the groom slot is **still occupied** by the in-flight groom dispatch. When the promoted groom wrapper goes to fire, it'll hit the per-class gate and re-defer. The 60s periodic backstop then catches up. Net effect: one wasted DeferredDispatch turn + 60s second-promotion latency.

**Why this plan defers it:**

- The ticket text explicitly scopes to `promote_pending_deferred_if_idle` ("The **periodic backstop** ... still promotes only ONE wrapper"). Folding the inline path expands scope.
- The inline-path failure is bounded: a single re-defer + one 60s retry — same throughput penalty the periodic backstop's class-blindness has today (which #1175 is fixing).
- Fixing both in one PR is mechanically cheap (the same DB primitives apply), but ratchets `Unit 3` from "add one sibling" to "modify the inline call site AND add a sibling" — a touch more risk-surface for a fix the operator labelled P2.

**Architect prompt:** Is folding the inline path into this PR justified? Cost: ~10 LOC + one extra DB primitive call site update. Benefit: removes the bounded-but-real inline race for cross-class concurrent dispatches. Counter: file as a P3 follow-up if you want #1175 to ship in narrow scope.

### Open question 2: Should the iteration order be deterministic-stable, or randomized?

Currently fixed order: `["implement", "groom"]`. Implications:

- **Fixed (current proposal):** Predictable. Pre-v34 NULL-class wrappers always processed first. Easy to reason about in tests.
- **Randomized per-tick:** Fairer when class counts diverge wildly (e.g., 100 implement wrappers and 1 groom). With fixed order, the implement class always gets first chance, but per-tick promotion is one-per-class so the groom wrapper still promotes in the same tick. No actual fairness gap at the per-tick-one-promote granularity.

**Architect prompt:** Lock fixed order, or open follow-up? I lean fixed order — randomization adds entropy without solving an observable problem.

### Open question 3: Single-class atomicity vs. cross-class atomicity?

The proposed engine loop is sequential per-class, no transaction wrapping the two slot checks. A new `implement` dispatch could land between the `implement` check and the `groom` check.

**Why a single transaction is unnecessary:** The two slot checks are independent (different rows, different classes). A new implement dispatch landing mid-loop is identical to the existing single-class behavior — the groom slot check is unaffected. Wrapping the two checks in a serializable transaction would be lock-friction with no correctness benefit.

**Architect prompt:** Confirm sequential-no-transaction is acceptable. Citation: same shape as the existing `engine.rs:431-440` check-then-promote (no transaction there either).

## Test Plan

| Test | Layer | Asserts |
|------|-------|---------|
| Test 1 (Unit 5) | DB | `promote_next_deferred_callback_for_class` filters by class; FIFO within class; NULL treated as `'implement'`. |
| Test 2 (Unit 5) | DB | `has_any_active_callback_for_class` is class-scoped; excludes `:deferred`; NULL handling via COALESCE. |
| Test 3 (Unit 6) | Engine | R1: two cross-class wrappers, both class slots idle → both promote in single tick. |
| Test 4 (Unit 6) | Engine | R3: cross-class wrappers, one class busy → only idle class promotes. |
| Test 5 (Unit 6) | Engine | `DISPATCH_CLASSES` slice covers all values returned by `derive_dispatch_class`. Drift detector. |

Existing tests retained, must stay green:

- `test_promote_next_deferred_callback_fifo` (db.rs:16257) — pre-existing FIFO test stays green (agent-wide method unchanged).
- `test_has_any_active_callback` (db.rs:16388) — pre-existing agent-wide active-callback test stays green.
- `test_has_active_callback_tasks_excluding_ignores_deferred_wrappers` (db.rs:16451) — mika#1163's slot-guard test stays green.
- All mika#1011 + mika#1070 + mika#1124 + mika#1163 tests retained.

**Acceptance gate:** `cargo test -p mika-agent` is green; `cargo clippy -p mika-agent` is clean.

## Coupled docs

- **`crates/mika-agent/CLAUDE.md`** — Update "Promotion paths (mika#1070)" bullet (Unit 7).
- **No `docs/solutions/` entry yet** — this is a follow-up to mika#1163's pattern. If the architect spots a generalizable lesson (class-aware predicates as a class of issue), file a compound writeup post-merge.

## Compound

After merge, the compound writeup should sit alongside the mika#1163 entry (which lives at `docs/solutions/architecture-patterns/` if filed, or under `database-issues/` per its slot-predicate framing). Frame: "class-aware predicates: when adding a class dimension to a system, audit all sibling predicates for symmetric class-awareness." mika#1163 and mika#1175 are the two halves of this pattern at the deferred-dispatch perimeter.

## Status

Ready for first-pass architect review.
