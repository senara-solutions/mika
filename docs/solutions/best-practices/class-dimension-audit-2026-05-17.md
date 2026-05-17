---
title: "Class-dimension audit: when adding a class column, audit all sibling predicates for symmetric class-awareness"
date: 2026-05-17
category: best-practices
module: task-engine
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - A new class/type/tier column has been added to a row that gates concurrency, dispatch, or scheduling (e.g., `dispatch_class`, `priority_tier`, `tenant_id`)
  - The system has multiple predicates that read the same rows for different decisions (gate vs. backstop, slot-check vs. promotion, eligibility vs. selection)
  - One predicate is being updated to honor the new class dimension but the others were left class-blind
  - A quality-of-service regression is suspected under cross-class load that the original single-class tests never exercised
tags:
  - dispatch-class
  - class-aware-predicates
  - predicate-audit
  - task-engine
  - autonomous-loop
  - symmetric-class-handling
  - structural-invariant
---

# Class-dimension audit: when adding a class column, audit all sibling predicates for symmetric class-awareness

## Context

When mika#1001 added the `dispatch_class` column (`'implement'` vs `'groom'`) to split the single global dispatch slot into two parallel class-scoped slots, the change was localized to the slot-guard predicate `has_active_callback_tasks_excluding()` (the predicate that decides "is the slot occupied? reject this dispatch"). The split unblocked one-implement-plus-one-groom concurrency for the autonomous loop.

The class column then leaked across two follow-up bugs:

- **mika#1163** — `has_active_callback_tasks_excluding` was per-class but missing `AND label NOT LIKE '%:deferred'`, while its sibling `has_any_active_callback` already excluded `:deferred` wrappers. Two parents each holding a pending `:deferred` wrapper deadlocked every cross-parent dispatch attempt. Fix: bring the slot-guard's `:deferred` exclusion in line with the sibling.
- **mika#1175** — `has_any_active_callback` and `promote_next_deferred_callback` were both agent-wide (class-blind), while the per-class slot guard was already class-scoped. The periodic backstop fired once per 60s tick and promoted exactly one wrapper agent-wide regardless of class, halving deferred throughput under cross-class load. Fix: add `_for_class` siblings to both DB primitives and iterate the engine backstop over the class universe.

Both bugs were quality-of-service regressions that only manifest under cross-class load, which is precisely the load shape mika#1001 unlocked. The bugs were not introduced by mika#1001 — they were *unmasked* by it. The class dimension had been added to one predicate; the sibling predicates that read the same row set were left as they were and silently lost the parity that the system depended on.

This document generalizes the audit a future implementer should run before declaring a class-dimension change complete.

## Guidance

When you add a class/type/tier column to a row that participates in concurrency, dispatch, or scheduling decisions, the column is not localized to the predicate you obviously had to change. Treat every sibling predicate that reads the same rows as part of the same change set, and decide explicitly for each whether it should:

1. **Become class-scoped** — the predicate's decision depends on the new dimension (e.g., "is THIS class's slot occupied").
2. **Stay class-blind** — the predicate's decision is genuinely cross-class (e.g., "is there any work at all for this agent").
3. **Be split into a pair** — both shapes are needed, so add the class-scoped sibling and keep the agent-wide form for the path that needs it.

The third case is the most common and the easiest to miss. Both forms must coexist because different callers want different scopes. The new code introduces a sibling alongside the existing one rather than replacing it.

**Concrete audit steps** for any class-dimension change:

1. **Grep all readers of the column.** Identify every predicate, query, or join that filters by `agent_id` (or whatever the existing scope key is) and selects from the same table. Each of those is a candidate for class-awareness.
2. **Classify each reader's decision intent.** Does this reader make a per-class decision (gate, slot, eligibility for this class)? An agent-wide decision (any work, total count)? Or both, depending on caller? Match the predicate shape to the intent.
3. **Pair the predicates with their callers.** A reader called from the per-class slot guard MUST be class-scoped. A reader called from a global "are we busy at all" backstop CAN stay class-blind, BUT verify that the backstop's *next action* (promotion, dispatch) is also class-aware — otherwise you've fixed the gate and left the engine downstream still class-blind, which is the mika#1175 shape.
4. **For every predicate kept class-blind, add a comment explaining why.** A future maintainer reading the file should not have to re-derive the decision; the asymmetry between sibling predicates needs a coupled-pair comment naming both sites (see `docs/solutions/architecture-patterns/asymmetric-perimeter-predicate-drift.md` for the general pattern and `docs/solutions/architecture-patterns/task-engine-success-side-parent-completion-backstop-2026-05-17.md` for the coupled-pair comment convention).
5. **Pin the class universe with a drift test.** The set of valid class values comes from a small Rust match (e.g., `derive_dispatch_class`). Add a test that asserts every output of that function is a member of the class slice the engine iterates over. The probe list is hand-maintained, but the test fails loud when a new class is added without updating the slice. See `crates/mika-agent/src/task_engine/engine.rs::test_dispatch_classes_universe_matches_derive_fn`.
6. **Verify the SET clause parity** when adding a class-scoped UPDATE sibling. The new UPDATE's SET columns must be identical to its agent-wide sibling — only the WHERE clause should differ. Downstream consumers (delivery scanners, completion handlers) key off the SET-side row state, not the WHERE-side selection.

## Why This Matters

**Class-dimension changes have asymmetric blast radius.** The change that introduced the class column (mika#1001) added one DB predicate and one CHECK constraint. Both follow-up bugs (mika#1163 and mika#1175) were quality-of-service regressions in *other* predicates that read the same rows and silently lost parity. The original change's tests passed; the new code's tests passed; the system as a whole regressed under cross-class load that the single-class regression suite never exercised.

The regression class is hard to detect because:

- It does not corrupt data. No row is lost; no constraint is violated.
- It does not throw errors. Every individual DB call returns the right answer for its narrow query.
- It only manifests under load shapes that the system was specifically modified to support — which means the original feature's tests don't cover it, and the autonomous loop doesn't reliably hit the load shape until production cadence saturates the now-split slots.

Three instances of this class are now documented (mika#1001 → mika#1163 → mika#1175). The pattern is real enough to deserve a standing audit checklist instead of being re-derived each time.

The cost of running the audit is low (one grep pass + one decision per matching predicate + one drift test). The cost of skipping it scales with the system's load and is invisible until a specific cross-class scenario starves.

## When to Apply

- Any PR that adds a new column to a table where existing predicates already filter by `agent_id`, `tenant_id`, `user_id`, or a similar partition key.
- Any PR that introduces a new dispatch class, priority tier, or scheduling lane that joins or partitions an existing concurrency-control table.
- Any code review touching `task_engine/`, `dispatcher.rs`, or other concurrency-gating code where a sibling predicate exists for the same row set.
- Any grooming pass for a follow-up ticket to a class-dimension change — even if the parent ticket appears complete, the audit may surface a sibling predicate that was left class-blind in error.

## Examples

### mika#1001 → mika#1163: sibling exclusion clause drift

`has_any_active_callback` (engine backstop, agent-wide) had `AND label NOT LIKE '%:deferred'` from mika#1070. `has_active_callback_tasks_excluding` (per-class slot guard, mika#1001) was added without the same exclusion. Both predicates read `tasks WHERE trigger_type = 'callback' AND status IN (...)` for the same agent, but only one applied the `:deferred` exclusion.

```rust
// has_any_active_callback (mika#1070) — had the exclusion
WHERE agent_id = ?1
  AND trigger_type = 'callback'
  AND status IN ('pending', 'in_progress')
  AND label NOT LIKE '%:deferred'   // <-- present

// has_active_callback_tasks_excluding (mika#1001 baseline)
WHERE trigger_type = 'callback'
  AND status IN ('pending', 'in_progress')
  AND agent_id = ?2
  AND COALESCE(dispatch_class, 'implement') = ?3
  // <-- missing the exclusion until mika#1163 added it
```

Audit would have caught it at step 1 (grep readers of the column → two predicates) + step 3 (both gate the same "slot occupied" concept; their exclusion sets must match).

### mika#1001 → mika#1175: agent-wide sibling left class-blind

`has_any_active_callback` and `promote_next_deferred_callback` (the engine backstop's check-then-promote pair, mika#1070) were both agent-wide. After mika#1001 split the slot per-class, the slot guard was per-class but the backstop kept picking one wrapper agent-wide per tick — cross-class wrappers serialized at 60s/promotion instead of 1-promotion-per-class-per-tick.

```rust
// engine.rs — pre-mika#1175 (class-blind)
async fn promote_pending_deferred_if_idle(&self) {
    match self.db.has_any_active_callback().await { ... }
    self.dispatcher.dispatch_next_deferred_callback().await;
}

// engine.rs — post-mika#1175 (class-aware)
async fn promote_pending_deferred_if_idle(&self) {
    for class in DISPATCH_CLASSES {
        match self.db.has_any_active_callback_for_class(class).await { ... }
        self.dispatcher.dispatch_next_deferred_callback_for_class(class).await;
    }
}
```

Audit would have caught it at step 3 (the backstop's next action — promotion — must be class-aware too) + step 5 (drift test pins the class universe).

### Decision matrix for a future class-column rollout

| Predicate intent | Action |
|------------------|--------|
| "Is THIS class's slot occupied?" (gate) | Class-scoped predicate, `COALESCE(class, default) = ?` clause |
| "Is there ANY work for this agent?" (global busy-check, e.g., GC eligibility, dashboard total) | Stay class-blind, add coupled-pair comment naming the per-class sibling |
| "Promote the next pending row" (selector that needs to honor class slots) | Class-scoped sibling; keep agent-wide form for whichever caller genuinely wants agent-wide selection (often: none — flag for follow-up) |
| "Count pending rows" (observability, capacity planning) | Usually class-aware; consider per-class breakdown in the return type |

## Related

- `docs/solutions/architecture-patterns/asymmetric-perimeter-predicate-drift.md` — general pattern for "same concept, two consumers, diverging sets." Class-dimension drift is one instance of the broader perimeter-predicate drift class.
- `docs/solutions/logic-errors/deferred-dispatch-promotion-deadlock-2026-05-10.md` — mika#1163 case study (slot-guard `:deferred` exclusion).
- `docs/solutions/best-practices/per-class-dispatch-slot-2026-05-11.md` — mika#1001 foundation (the per-class slot split itself).
- `docs/solutions/architecture-patterns/task-engine-success-side-parent-completion-backstop-2026-05-17.md` — coupled-pair comment convention referenced in step 4.
- mika#1175 PR — the periodic-backstop class-awareness fix.
- mika#1175 plan — `docs/plans/2026-05-17-002-chore-1175-task-engine-deferred-backstop-class-aware-plan.md`
