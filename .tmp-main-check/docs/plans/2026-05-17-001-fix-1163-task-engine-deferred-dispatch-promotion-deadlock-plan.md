---
title: "fix(task-engine): per-class slot predicate must exclude deferred wrappers (mika#1163)"
type: fix
status: active
date: 2026-05-17
---

# fix(task-engine): per-class slot predicate must exclude deferred wrappers (mika#1163)

## Overview

`has_active_callback_tasks_excluding()` — the per-class dispatch-slot predicate used by `validate_dispatch_readiness()` — counts pending `:deferred` wrappers as active dispatches. When two parents each have a pending deferred wrapper, every dispatch attempt from either wrapper sees the OTHER wrapper as "slot occupied", registers a NEW wrapper, and the queue deadlocks. Fix is one SQL clause: add `AND label NOT LIKE '%:deferred'` so the predicate matches the sibling `has_any_active_callback()` semantics that the periodic-backstop already uses.

## Problem Frame

### What goes wrong (operator-visible)

After mika#1158's stuck parent task was cancelled at 23:43Z on 2026-05-17, three deferred tickets (mika#1155, mika#797, mika#859) failed to promote despite the dispatch slot being logically idle (zero claude-pilot subprocesses, zero `in_progress` callbacks). mika-dev's message log cycled every ~60 seconds:

```
21:55:00Z Deferred again — mika#1155 (task 394013df) still has an active dispatch. mika#797 re-queued automatically.
21:56:01Z Deferred again — mika#797 (task f6f03d7a) still has an active dispatch (f8aeb7e1). mika#1155 re-queued automatically.
21:56:59Z Deferred again — mika#1155 (task 394013df) still active. mika#797 will auto-retry when that slot frees.
21:57:59Z Deferred again — mika#797 (task f6f03d7a, callback fdcec011) still active. mika#1155 re-queued automatically.
```

Each iteration cited a different callback ID. At 23:58:06Z, pending callbacks were `86587586` + `26256c63` + `10783a3f` with zero claude-pilot processes and zero `in_progress` callbacks — three deferred wrappers cycling against each other forever.

### Why prior fixes don't catch this

- **mika#1070** added `promote_pending_deferred_if_idle()` as a 60-second periodic backstop that uses `has_any_active_callback()` (which DOES exclude `:deferred` labels — `db.rs:5839`). The backstop correctly detects slot-idle and promotes one wrapper per tick.
- **mika#1124** re-added the inline anti-cascade guard at `dispatcher.rs:495` to prevent inline chain-promotion when a deferred-wrapper completes without actually dispatching. That guard is orthogonal to this bug.
- **mika#1058** wired `LongRunningContext` into `DeferredDispatch` turns so the LLM CAN call `run_claude_pilot` from the silent retry path. Without #1058 the deferred turn would silently fail at the gate; with it, the turn correctly reaches `run_claude_pilot`.

The remaining gap is in the OTHER slot-check path: `validate_dispatch_readiness()` at `executor.rs:946` calls `has_active_callback_tasks_excluding(task_id, class)` (`db.rs:5752`), and THAT predicate has no `:deferred` exclusion. So while the backstop correctly promotes a wrapper, the promoted wrapper's DeferredDispatch turn is rejected by the gate the moment it tries to dispatch — because the gate sees OTHER pending wrappers (from OTHER parents) as "active dispatches in this class".

### Trace of the deadlock (with file:line evidence)

State: parents A, B each have one pending wrapper (`label='long_running:run_claude_pilot:deferred'`, `dispatch_class='implement'`). Zero `in_progress` non-deferred callbacks.

1. Engine tick fires `promote_pending_deferred_if_idle()` (`engine.rs:234`).
2. `has_any_active_callback()` (`db.rs:5839`) — query has `label NOT LIKE '%:deferred'` — returns `false`. ✓
3. `dispatch_next_deferred_callback()` → `promote_next_deferred_callback()` (`db.rs:5812`) → A's wrapper UPDATE: `status='completed'`.
4. `dispatch_undelivered_callbacks()` (`engine.rs:356`) picks up the just-promoted wrapper, calls `dispatch_resume_agent()`.
5. `dispatch_resume_agent` (`dispatcher.rs:301`) sees `task.label == DEFERRED_DISPATCH_LABEL` → uses `SilentTrigger::DeferredDispatch` (`dispatcher.rs:308`).
6. Silent agent runs; INTENT_GUARD (`deferred_dispatch_action`) requires `run_claude_pilot` call.
7. LLM calls `run_claude_pilot` → `execute_long_running` → `validate_dispatch_readiness(parent_A_id, ...)` (`executor.rs:1620`).
8. Inside, `has_active_callback_tasks_excluding(parent_A_id, "implement")` (`executor.rs:946`).
9. The SQL at `db.rs:5752`:
   ```sql
   SELECT parent_task_id, id FROM tasks
   WHERE trigger_type = 'callback'
     AND status IN ('pending', 'in_progress')
     AND parent_task_id IS NOT NULL
     AND parent_task_id != ?1
     AND agent_id = ?2
     AND COALESCE(dispatch_class, 'implement') = ?3
   LIMIT 1
   ```
   No `label NOT LIKE '%:deferred'` clause. Matches B's pending deferred wrapper (parent != A, class = implement, status = pending).
10. Returns `Some((B_parent_id, B_wrapper_id))` → validate_dispatch_readiness returns `global_dispatch_active` error → `register_deferred_callback()` (`executor.rs:1487`) creates ANOTHER pending wrapper for A.
11. Symmetric for B's promotion cycle. Net pending-wrapper count stays at ~2 (or bounded by `MAX_PENDING_DEFERRED_CALLBACKS = 10`, `executor.rs:1479`); no real dispatch ever spawns.

### Why the bug only surfaces with ≥2 deferred parents

When only ONE parent has a pending wrapper, the `parent_task_id != ?1` clause in `has_active_callback_tasks_excluding` excludes that parent's OWN wrapper. The query returns `None`, validate passes, and the DeferredDispatch turn dispatches successfully.

When TWO parents both have pending wrappers, each parent's gate-check finds the OTHER parent's wrapper as a "blocking dispatch". Both lose.

The bug went undetected through mika#1011 (initial deferred-dispatch primitive), mika#1070 (first deadlock fix), mika#1058 (DeferredDispatch+long_running wiring), and mika#1124 (re-added anti-cascade guard) because all of those tested scenarios involved at most one pending wrapper at a time. Today's load (3 ready-labeled tickets in parallel) is the first time the multi-wrapper case surfaced.

### Why this is NOT downstream of mika#1162

The issue body speculated this might be downstream of mika#1162 (parent task not auto-transitioned after callback delivery). It is not. `has_active_callback_tasks_excluding` queries the `tasks` table for **callback children**, not for the parent's status. Whether the parent is stuck in `in_progress` (mika#1162) or correctly transitioned to `completed` is irrelevant to this predicate. The two bugs are independent and can be fixed independently.

## Requirements Trace

- **R1.** Two ready-labelled tickets dispatched concurrently → one runs, one defers. When the first completes (callback delivered), the second auto-promotes within 60 seconds. (AC1 from issue body — already satisfied by the periodic backstop; this fix ensures the promoted wrapper actually dispatches instead of re-deferring.)
- **R2.** Cancellation of a stuck parent triggers immediate deferred-promotion of a sibling wrapper. With this fix, the next 60s backstop tick promotes one wrapper successfully. (AC2 from issue body. "Immediate" interpreted as "within one DB_SCAN_INTERVAL_TICKS cycle" — same bound as R1.)
- **R3.** Per-class slot guard (`has_active_callback_tasks_excluding`) treats `:deferred` wrappers as markers, not as active dispatches. (AC3 from issue body — restated structurally.)
- **R4.** Regression test reproduces the multi-wrapper deadlock and verifies the fix.
- **R5.** No regression in single-wrapper scenarios (mika#1011 design), in cross-class concurrency (mika#1001 per-class slot split), or in pre-v34 NULL dispatch_class handling.

## Scope Boundaries

- **In scope:** The one SQL clause in `has_active_callback_tasks_excluding` (db.rs:5752). Regression tests in `crates/mika-agent/src/skills/executor.rs` (where the existing slot-guard tests live) AND in `crates/mika-agent/src/db.rs` (where the existing `has_any_active_callback` regression test lives). Brief CLAUDE.md note in the **Unified Task Engine** section so the predicate's `:deferred` exclusion is documented next to its sibling.
- **Out of scope:**
  - **mika#1162** (parent task not auto-transitioned). Independent root cause; separate ticket.
  - **mika#1124** anti-cascade guard at `dispatcher.rs:495`. Orthogonal — that guard governs INLINE chain-promotion after a wrapper's silent turn completes; this fix governs the GATE predicate. Both can coexist.
  - **mika#1126**, **mika#1118** reaper bugs. Listed as related in the issue body but not implicated in this deadlock trace.
  - **mika-dev prompt changes.** The "Deferred again" message is the LLM correctly relaying the structured `global_dispatch_active` rejection. Fixing the predicate makes the rejection stop firing; no prompt change needed.
  - **Webhook deferral queue** (`webhook_queue.rs:126`'s `has_active_callback_child`). Its scope is per-task (`get_child_tasks(task_id)`), not cross-parent. Its semantics are correct: a parent with a pending wrapper IS waiting in the dispatch pipeline, so deferring its webhooks is defensible. Once this primary bug is fixed and wrappers promote reliably, secondary webhook-queue behavior heals.
  - **`MAX_PENDING_DEFERRED_CALLBACKS = 10`** flood cap. The cap is correct safety; it isn't the deadlock vector. Once wrappers can dispatch, the cap rarely matters.

## Context & Research

### Relevant Code and Patterns

- **`crates/mika-agent/src/db.rs:5752`** — `has_active_callback_tasks_excluding`. The single fix site.
- **`crates/mika-agent/src/db.rs:5839`** — `has_any_active_callback`. Already has the correct `label NOT LIKE '%:deferred'` exclusion. The sibling pattern this fix mirrors.
- **`crates/mika-agent/src/skills/executor.rs:946`** — `validate_dispatch_readiness` call-site that drives the deadlock.
- **`crates/mika-agent/src/task_engine/dispatcher.rs:495`** — `mika#1124` anti-cascade inline guard. Stays as-is.
- **`crates/mika-agent/src/task_engine/engine.rs:423`** — `promote_pending_deferred_if_idle` backstop. Correctly uses `has_any_active_callback`. Stays as-is.
- **`crates/mika-agent/src/skills/executor.rs:3933-4115`** — existing per-class slot-guard tests. The new regression tests live in the same `mod tests` block.
- **`crates/mika-agent/src/db.rs:15880-15933`** — `test_has_any_active_callback`. The structural template for the new `has_active_callback_tasks_excluding` deferred-wrapper test.

### Institutional Learnings

- `docs/solutions/logic-errors/deferred-dispatch-promotion-deadlock-2026-05-10.md` (mika#1070). Same module, three-defect class. This fix adds a fourth structural defect (asymmetric `:deferred` exclusion across the two slot predicates) that was not surfaced in 2026-05-10's analysis. **Will update on compound:** add an "Update 2026-05-17 (mika#1163)" section describing the asymmetric-predicate class.
- `docs/solutions/logic-errors/callback-deferred-dispatch-gate-rejection-2026-05-10.md` (mika#1058). Same gate (`validate_dispatch_readiness`), different gap (DeferredDispatch turns being blocked from `run_claude_pilot`). Provides the call-flow context.
- `docs/solutions/architecture-patterns/phantom-retry-guard-active-dispatch-metadata-validation.md`. Pattern reference for "active callback" classification — confirms the convention that `:deferred` wrappers are NOT "active dispatches" semantically.
- `docs/solutions/architecture-patterns/webhook-deferral-queue-callback-sequencing.md`. Companion module reference; confirms scope boundary against `has_active_callback_child`.

### Workspace memory consulted

- `feedback_check_code_when_asked_about_code` — every claim above is grounded in file:line evidence from the read code, not the issue body's hypothesis.
- `feedback_pipeline_match_severity` — surgical 1-line fix matches the p1-critical, well-scoped nature of the bug. Doesn't bundle adjacent improvements.
- `feedback_implementation_scope_bundling` — explicitly excluded mika#1162, mika#1124, mika#1126, mika#1118 even though they're listed as related. Each is a separate ticket.
- `feedback_compound_infra_fixes` — this is an infra fix. Compound after PR. Look back at the 2026-05-10 plans before shipping a new one (done: read both 2026-05-10-* plans + solution doc).

## Key Technical Decisions

- **Surgical 1-clause SQL fix, not a refactor.** The two slot predicates (`has_active_callback_tasks_excluding`, `has_any_active_callback`) have similar but not identical shapes — one is per-class+excluding-self, the other is agent-wide. They could be unified into a single predicate with parameters, but that's a refactor and risks bundling regressions into a p1 hot-fix. Defer the unification to a follow-up if/when a third caller emerges.
- **Add `AND label NOT LIKE '%:deferred'` (not `AND label != 'long_running:run_claude_pilot:deferred'`).** Mirrors the exact pattern in `has_any_active_callback` (db.rs:5846) so a grep on `:deferred` lands on every site that needs to evolve together if the suffix convention ever changes. Wildcard match also covers any future deferred-label variants (e.g., per-skill `:deferred:groom`).
- **Drift-guard for label suffix.** mika#1124 added a drift guard at `dispatcher.rs:2339` asserting `DEFERRED_DISPATCH_LABEL` ends in `:deferred`. The new SQL clause depends on that suffix convention; the drift guard already covers it. No new guard needed.
- **Tests live in both modules.** Put the multi-wrapper deadlock reproduction in `crates/mika-agent/src/db.rs` (next to the `has_any_active_callback` regression test, mirroring the same `mika#1070` block) AND a higher-level scenario test in `crates/mika-agent/src/skills/executor.rs` (next to the existing `has_active_callback_tasks_excluding` tests). Two-layer coverage: DB-level isolated unit test + skill-level integration that exercises through `async_db`.
- **Documentation update is brief.** Add one bullet in `crates/mika-agent/CLAUDE.md` near the existing **Unified Task Engine** section explaining that BOTH slot predicates exclude `:deferred` wrappers — so future contributors don't reintroduce the asymmetry.

## Open Questions

### Resolved During Planning

- **Should we also fix mika#1124's inline guard?** No. mika#1124's guard correctly prevents inline chain-cascade when a wrapper's silent turn no-ops; it doesn't drive this deadlock. (`dispatcher.rs:495` only fires on the inline path after `mark_task_delivered`; the deadlock is in the GATE predicate path.)
- **Should the wrapper completing trigger an immediate webhook_queue drain?** Out of scope (see Scope Boundaries). Becomes irrelevant once wrappers actually dispatch.
- **Should we add a more aggressive backstop (faster than 60s)?** No. The 60s cadence is the documented contract from mika#1070 (`DB_SCAN_INTERVAL_TICKS`). The deadlock isn't caused by backstop slowness; it's caused by the gate rejecting promoted wrappers. With the fix, the existing 60s cadence satisfies AC1 and AC2.
- **Should `promote_next_deferred_callback` be invoked synchronously inside `cancel_task` for "immediate" recovery?** Could be a nice-to-have, but AC says "within 60 seconds" elsewhere in the issue. The periodic backstop already meets the bound. Adding a synchronous hook would couple `cancel_task` to the dispatcher (currently it's a pure DB write). Defer.

### Deferred to Implementation

- **Exact assertion shape for the integration test.** Whether to assert on rejection-string content (`"global_dispatch_active"`) absence, or assert on the `Ok` return of `validate_dispatch_readiness`, depends on which is more stable. Decide while writing.

## Implementation Units

- [ ] **Unit 1: Add `:deferred` exclusion to per-class slot predicate**

**Goal:** Fix the SQL predicate so deferred wrappers do not count as active dispatches in the per-class slot check.

**Requirements:** R3, R1, R2.

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (the SQL string in `has_active_callback_tasks_excluding` at line 5752)

**Approach:**
- Add a single clause `AND label NOT LIKE '%:deferred'` to the existing query, positioned after `AND COALESCE(dispatch_class, 'implement') = ?3` for parallelism with the sibling predicate's clause ordering.
- Update the doc-comment above the function to call out that the exclusion mirrors `has_any_active_callback` and cite mika#1163.
- Do NOT change the function signature, return type, or call sites.

**Patterns to follow:**
- `crates/mika-agent/src/db.rs:5846` — `has_any_active_callback`'s exact `AND label NOT LIKE '%:deferred'` clause.

**Test scenarios:**
- Happy path: see Unit 2.

**Verification:**
- Function signature unchanged; SQL clause appears exactly once; doc-comment cites mika#1163.

- [ ] **Unit 2: DB-level regression test for `:deferred` exclusion**

**Goal:** Pin the per-class slot predicate's `:deferred` exclusion with a sibling-shape test to the existing `test_has_any_active_callback` (db.rs:15883).

**Requirements:** R4.

**Dependencies:** Unit 1.

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (add `#[test] fn test_has_active_callback_tasks_excluding_ignores_deferred_wrappers` in the existing `mod tests` block, placed adjacent to `test_has_any_active_callback`)

**Approach:**
- Mirror the structure of `test_has_any_active_callback` (db.rs:15883). Create one parent, register a `:deferred` callback child, assert the predicate returns `None` for any other-parent query. Then add a real (non-`:deferred`) callback for a second parent and assert the predicate returns `Some(...)` for queries from a third parent (proving the exclusion only kicks in for `:deferred` rows, not all rows).

**Patterns to follow:**
- `db.rs:15883-15933` — `test_has_any_active_callback` template (state setup → predicate query → assert → mutate → re-assert).

**Test scenarios:**
- **Happy path: deferred wrappers excluded** — One parent + one `:deferred` pending wrapper. Querying with any other parent_id returns `None`. (Proves the exclusion clause.)
- **Happy path: real callbacks still detected** — Add a second parent with a real (non-`:deferred`) pending callback. Querying with a third parent_id returns `Some((second_parent_id, real_callback_id))`. (Proves the exclusion is wrapper-only, not blanket.)
- **Edge case: mixed deferred + real on same agent** — Two parents, one with a `:deferred` wrapper, one with a real pending callback. Querying with a third parent_id returns the REAL callback's parent (not the wrapper's). (Proves the exclusion does the right thing in mixed states.)
- **Edge case: in_progress deferred wrapper** — Although currently no code path sets a `:deferred` wrapper to `in_progress`, ensure the SQL `LIKE` pattern catches both pending AND in_progress `:deferred` rows. (Forward-compat: if a future code path ever flips a wrapper to in_progress, the exclusion still holds.)

**Verification:**
- `cargo test -p mika-agent test_has_active_callback_tasks_excluding_ignores_deferred_wrappers` passes.
- The test reads as a clear sibling to `test_has_any_active_callback` (same shape, same agent name, same helper functions).

- [ ] **Unit 3: Skill-level integration test reproducing the multi-wrapper deadlock**

**Goal:** Reproduce the mika#1163 deadlock scenario end-to-end through `validate_dispatch_readiness` and assert the fix unblocks both wrappers.

**Requirements:** R4, R5.

**Dependencies:** Unit 1.

**Files:**
- Modify: `crates/mika-agent/src/skills/executor.rs` (add `#[tokio::test(flavor = "multi_thread", worker_threads = 2)] async fn test_per_class_slot_does_not_block_on_deferred_wrappers` in the existing `mod tests` block, placed adjacent to the existing per-class slot tests, around line 4083)

**Approach:**
- Create two parent tasks (A, B) in `in_progress` status.
- Use the existing `create_callback_child_with_class` helper to register one pending `:deferred` wrapper under each parent (need a small variant or inline call that sets `label = "long_running:run_claude_pilot:deferred"`).
- Call `async_db.has_active_callback_tasks_excluding(&A, "implement").await.unwrap()`.
- Assert: returns `None` (B's wrapper is no longer counted as a blocking dispatch).
- Symmetric assertion for B.
- Then create a real (non-`:deferred`) callback under a third parent C and re-assert that querying with A returns `Some((C, ...))` (the real callback IS detected; only wrappers are excluded).

**Patterns to follow:**
- `executor.rs:4029` — `test_per_class_slot_allows_different_class_concurrent` (test shape: two parents, callback children, assertion through `async_db`).
- `executor.rs:3987` — `create_callback_child_with_class` helper.

**Test scenarios:**
- **Happy path: two deferred wrappers, no mutual blocking** — Per the approach above.
- **Edge case: deferred wrapper + real callback** — Mixed-state assertion as above.
- **Edge case: NULL dispatch_class deferred wrapper** — Pre-v34 row with NULL dispatch_class + `:deferred` label. The COALESCE+label exclusion both apply — verify it's still excluded.

**Verification:**
- `cargo test -p mika-agent test_per_class_slot_does_not_block_on_deferred_wrappers` passes.
- The test reproduces the deadlock by REVERTING Unit 1 locally (sanity-check during dev) — should fail without the fix, pass with it.

- [ ] **Unit 4: Doc note in CLAUDE.md**

**Goal:** Document the symmetric `:deferred` exclusion across both slot predicates so a future contributor doesn't reintroduce the asymmetry.

**Requirements:** R5 (forward-compat).

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` (the **Unified Task Engine** section's **Promotion paths (mika#1070)** bullet — extend it with a one-sentence note)

**Approach:**
- Append to the existing mika#1070 promotion-paths bullet: "Both slot predicates exclude `:deferred` wrappers (the perimeter must agree across `has_any_active_callback` for the backstop AND `has_active_callback_tasks_excluding` for `validate_dispatch_readiness` — mika#1163 was the deadlock from an asymmetric exclusion)."
- No other docs changes. Solution doc creation happens in the post-PR compound step, not in this implementation.

**Patterns to follow:**
- The CLAUDE.md style of inline ticket annotations (e.g., `(mika#1070)`, `(#1011)`).

**Test scenarios:**
- Test expectation: none — pure prose change in CLAUDE.md.

**Verification:**
- `grep -n "mika#1163" crates/mika-agent/CLAUDE.md` finds the new note.
- The note is one sentence, in the existing bullet, not a new section.

## System-Wide Impact

- **Interaction graph:** `validate_dispatch_readiness` (executor.rs:834) is called from `execute_long_running` (the long-running tool handler) and indirectly from every `run_claude_pilot` invocation. The fix narrows when `global_dispatch_active` fires — it stops firing in the multi-wrapper case. No other dispatch paths affected.
- **Error propagation:** No new error variants. Existing `global_dispatch_active` JSON error shape unchanged. Existing `register_deferred_callback` flow unchanged for the single-wrapper case (still fires when a REAL callback is in flight).
- **State lifecycle risks:** None. The fix only narrows the predicate; it does not change any state transition. Pre-existing pending wrappers in a wedged DB will start dispatching correctly within one tick cycle after the fix deploys.
- **API surface parity:** `has_any_active_callback` (engine-side, agent-wide) and `has_active_callback_tasks_excluding` (gate-side, per-class+excluding-self) now both exclude `:deferred`. Symmetric — the guard's "what counts as occupied" answer is consistent across both call sites.
- **Integration coverage:** Unit 3's `tokio::test(multi_thread)` integration test through `async_db` exercises the same code path as production. The post-deploy operational signal (deferred-wrapper count converges to 0 within 60s after slot becomes idle) confirms behavior in vivo.
- **Unchanged invariants:**
  - mika#1011 single-session-at-a-time guard (one REAL callback per dispatch_class) unchanged — `has_active_callback_tasks_excluding` still detects real callbacks.
  - mika#1001 per-class slot split (one implement + one groom concurrently) unchanged — `dispatch_class` filter preserved.
  - mika#1124 inline anti-cascade guard at `dispatcher.rs:495` unchanged — orthogonal path.
  - mika#1070 periodic backstop unchanged — sibling predicate already correct.
  - Pre-v34 NULL `dispatch_class` coercion (`COALESCE → 'implement'`) unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Hidden caller expects the predicate to count `:deferred` wrappers. | Greped all call-sites (`grep -nE has_active_callback_tasks_excluding`): only `validate_dispatch_readiness` (executor.rs:946) and the existing test fixtures + `ci_failure_handler.rs:301`. `ci_failure_handler` uses it for the same purpose — detecting a real active dispatch before triggering a fix-loop. The `:deferred` exclusion is correct in that context too: a fix-loop should not be blocked by pending deferred wrappers. |
| New SQL clause has a typo or wrong wildcard. | Unit 2 + Unit 3 tests pin the exact behavior. The clause is a verbatim copy of `db.rs:5846`. |
| Suffix convention `:deferred` changes in the future without updating both predicates. | mika#1124's drift-guard at `dispatcher.rs:2339` asserts the label ends in `:deferred`; any change to the constant fires the assertion. No additional drift guard needed. |
| The fix unmasks a downstream bug (e.g., DeferredDispatch turn produces unexpected state). | Existing scenario tested in production via mika#1011 and mika#1058 — the single-wrapper case has been working. This fix extends the working path to the multi-wrapper case. |
| Existing tests in `mod tests` block at executor.rs:3933-4115 break due to misuse of the helper. | The helper `create_callback_child_with_class` uses `format!("long_running:run_claude_pilot:{dispatch_class}")` — so it never produces a `:deferred` label. Existing tests are unaffected. |

## Documentation / Operational Notes

- **Operational signal (post-deploy):** On the first restart after deploy, in any DB with wedged pending wrappers, `grep deferred_dispatch_promoted` in `MIKA_SPIRIT_LOG_FILE` should show wrappers promoting and the corresponding DeferredDispatch turns successfully calling `run_claude_pilot` (NO `global_dispatch_active` rejection in the immediately following dispatch attempt). Real callbacks appear in `tasks` rows with `label = 'long_running:run_claude_pilot:implement'` (or `:groom`) and `status = 'in_progress'` + `process_id` set. Within ~60s of slot-idle, the pending-wrapper count converges to 0 (one promotion → one real dispatch per cycle, FIFO).
- **No DB migration.** Pure code change. Existing pending wrappers are unblocked by the next backstop tick after deploy.
- **No deploy ordering constraint.** Single-binary fix in `mika-agent`. `make deploy` from this branch is the deploy.
- **`/ce:compound` follow-up note (out of scope for this PR — compound step handles it).**
  - Add an "Update 2026-05-17 (mika#1163)" section to `docs/solutions/logic-errors/deferred-dispatch-promotion-deadlock-2026-05-10.md` describing the asymmetric-predicate failure class as a fourth defect not surfaced in the original analysis.
  - Consider promoting the asymmetric-perimeter learning to `docs/solutions/architecture-patterns/`: when two predicates encode the "same" concept (slot occupancy) for two different consumers (engine backstop + tool-boundary gate), their inclusion/exclusion sets MUST be identical, or the asymmetry creates a deadlock that the symmetric tests of either predicate alone won't catch.

## Sources & References

- Issue: senara-solutions/mika#1163 — body provides today's incident trace (2026-05-17 23:43Z onwards).
- Related solution: `docs/solutions/logic-errors/deferred-dispatch-promotion-deadlock-2026-05-10.md` (mika#1070).
- Related solution: `docs/solutions/logic-errors/callback-deferred-dispatch-gate-rejection-2026-05-10.md` (mika#1058).
- Related solution: `docs/solutions/architecture-patterns/phantom-retry-guard-active-dispatch-metadata-validation.md` (mika#579 — pattern reference for "active dispatch" classification).
- Related plan: `docs/plans/2026-05-10-003-fix-deferred-dispatch-promotion-deadlock-plan.md` (the mika#1070 plan whose backstop this fix completes).
- Related plan: `docs/plans/2026-05-10-001-bug-callback-safe-deferred-dispatch-plan.md` (the mika#1058 plan that wired the DeferredDispatch+long_running path).
- Code: `crates/mika-agent/src/db.rs:5752` (the fix site) and `crates/mika-agent/src/db.rs:5839` (the sibling pattern).
- Code: `crates/mika-agent/src/skills/executor.rs:946` (the gate call site driving the deadlock).
- Code: `crates/mika-agent/src/task_engine/dispatcher.rs:495` (mika#1124 inline anti-cascade — orthogonal, unchanged).
- Code: `crates/mika-agent/src/task_engine/engine.rs:234` (periodic backstop — unchanged).
- Related issues (NOT bundled here): mika#1162 (parent task not auto-transitioned), mika#1126 (reaper-on-groom-race), mika#1118 (reaper dispatch_class blindness).
