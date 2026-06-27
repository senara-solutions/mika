---
title: "Asymmetric perimeter predicate drift — same concept, two consumers, diverging sets"
date: 2026-05-17
category: architecture-patterns
module: agent-loop
problem_type: bug_fix
component: tooling
severity: high
applies_when:
  - Two predicates encode the same concept (e.g., "slot occupied", "unauthorized dispatch", "task busy") for two different consumers (engine backstop + tool-boundary gate, or pre-hoc gate + post-hoc EndTurn guard)
  - One predicate's inclusion/exclusion set may diverge from the other under refactor, follow-up fix, or label-convention change
  - A reviewer is about to ship a new structural guard, gate, or backstop that shares a domain concept with an existing predicate
tags:
  - asymmetric-perimeter
  - predicate-sharing
  - guard-registry
  - tool-boundary-gate
  - endturn-guard
  - structural-invariant
  - deadlock
  - false-positive
---

# Asymmetric perimeter predicate drift

## Context

Several Mika subsystems use a **two-perimeter** pattern: a pre-hoc tool-boundary gate (or engine backstop) blocks an action from starting, AND a post-hoc EndTurn guard (or alternate gate) catches the same shape from a different vantage point. Defense-in-depth. The two perimeters answer the same conceptual question — "is this dispatch authorized?", "is the slot occupied?", "is the task busy?" — but each implements its own predicate.

When the two predicates' inclusion/exclusion sets are written independently, they drift. The drift creates one of two failure modes:

| Drift direction | Failure mode | Example |
|---|---|---|
| Pre-hoc more permissive than post-hoc | False-positive corrections / noise — post-hoc fires on turns the pre-hoc already passed | mika#910 EndTurn `webhook_no_unauthorized_dispatch` matched PR review events that the tool-boundary correctly excluded |
| Post-hoc more permissive than pre-hoc | Deadlock or escape — the post-hoc backstop never fires when its dependency requires the pre-hoc to be wider | mika#1163: engine-backstop excluded `:deferred` wrappers but tool-boundary did not, so promoted wrappers were rejected at dispatch |

Both directions are bugs. The pattern is invisible inside either predicate's own test suite, because tests of one predicate don't construct the cross-perimeter state that exposes the drift.

## Documented instances

### 1. mika#910 — webhook unauthorized-dispatch guard pair

Two predicates encoding "is this turn an unauthorized `[GitHub]` webhook dispatch?":

- **Tool-boundary gate** (`crate::webhook_dispatch::is_unauthorized_webhook_dispatch`) — tight allowlist that correctly excluded ready-label, PR review, and check-suite events.
- **EndTurn INTENT_GUARD trigger** (`webhook_no_unauthorized_dispatch_trigger`) — broad predicate `msg.starts_with("[GitHub]") && !msg.starts_with(READY_LABEL_DISPATCH_MARKER)` that matched ALL `[GitHub]` events except ready-label, INCLUDING legitimate qa/ci skill territory.

Drift direction: post-hoc more permissive than pre-hoc → false-positive EndTurn corrections on PR and check-suite turns. See `docs/solutions/architecture-patterns/intent-guard-predicate-sharing-2026-05-14.md`.

**Fix:** Delegate the trigger to the same shared function: `is_unauthorized_webhook_dispatch(msg)`. Single source of truth.

### 2. mika#1163 — per-class dispatch slot guard pair

Two predicates encoding "is the per-class dispatch slot occupied?":

- **Tool-boundary gate** (`Database::has_active_callback_tasks_excluding`, `crates/mika-agent/src/db.rs:5752`) — per-class scoped, EXCLUDING the requesting parent's own callback. Did NOT exclude `:deferred` wrappers.
- **Engine backstop** (`Database::has_any_active_callback`, `crates/mika-agent/src/db.rs:5839`) — agent-wide, label-filtered. Correctly excluded `:deferred` wrappers via `AND label NOT LIKE '%:deferred'` (fixed in mika#1070).

Drift direction: backstop more permissive than gate → backstop correctly identified slot-idle and promoted a wrapper, but the gate then rejected the promoted wrapper's `run_claude_pilot` call (saw OTHER parents' wrappers as occupants), re-creating another wrapper. Deadlock between two pending wrappers from different parents.

**Fix:** Add the SAME exclusion clause to the gate predicate:

```sql
AND label NOT LIKE '%:deferred'
```

See `docs/solutions/logic-errors/deferred-dispatch-promotion-deadlock-2026-05-10.md` § "Update 2026-05-17 (mika#1163)".

### 3. mika#525 vs mika#549 (implicit) — active-dispatch check pair

Two predicates encoding "does this task have an active callback child?":

- **`validate_dispatch_readiness` check (2)** (`executor.rs:903-928`) — rejects dispatch when task has any callback child in `pending`/`in_progress`.
- **`callback-task-loop-prevention`** guards (mika#549, see `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`) — separate set of rules preventing callback turns from spawning new long-running tasks.

These two perimeters answer overlapping questions and have evolved independently across multiple tickets (mika#525, mika#549, mika#1058). They have not yet drifted into a bug, but the structural shape is the same as #910 and #1163 — two perimeters, separate predicates, no shared function or parity test. Worth tracking as a pre-incident risk.

### 4. mika#1576 — required_tools coherence across N layers (generalization to >2 perimeters)

The same concept ("does this `required_tools` token resolve to a real tool?") is enforced at four layers: build-time `verify_bundled_skills` checks 4/5, the per-turn `#516` gate, and the new load-time `apply_required_tools_coherence_check`. The first review pass copied the surface-builder and the `mcp__` exemption predicate verbatim into both the runtime check and the `mika skills validate` CLI — the same drift seed, caught before merge. Fix: extract `effective_tool_surface` + `required_tool_resolves` (one home for the predicate); reuse the parity-guarded `BUILTIN_TOOL_NAMES` constant. This instance generalizes the two-perimeter pattern to N layers and adds three sub-rules (vantage-dependent severity, fixpoint eviction, coherence-scope ≠ enforcement-scope) — see `docs/solutions/architecture-patterns/multi-layer-structural-invariant-shared-primitive-2026-06-27.md`.

## The pattern

```
+---------------------+        +-----------------------+
|  Pre-hoc / Gate     |        |  Post-hoc / Backstop  |
|  (tool boundary)    |        |  (EndTurn / engine)   |
+---------+-----------+        +-----------+-----------+
          |                                |
          | predicate_a(state)             | predicate_b(state)
          v                                v
   "is the action ok?"              "is the action ok?"
          |                                |
          +--------------+-----------------+
                         |
                         v
             concept: same question
             implementation: divergent
             tests: each predicate in isolation
             result: drift undetected until
                     a state arises that crosses
                     both perimeters
```

The drift-detection gap: each predicate's test suite constructs **its own** state shape and asserts **its own** answer. Neither suite constructs the state where BOTH perimeters fire and would disagree. Cross-reviewer agreement during code review is the most reliable detection (mika#1163's review caught the drift via three reviewers independently flagging the same missing structural test).

## Fixes (canonical → fallback)

### Canonical: shared-function delegation (mika#910's approach)

When both predicates are pure functions over the same input type, extract a single function and have both consumers call it:

```rust
// crate::webhook_dispatch
pub fn is_unauthorized_webhook_dispatch(msg: &str) -> bool {
    msg.starts_with("[GitHub]") &&
    !msg.starts_with(READY_LABEL_DISPATCH_MARKER) &&
    !is_pr_review_event(msg) &&
    !is_check_suite_event(msg)
}

// Tool-boundary gate (executor.rs)
if let Some(msg) = originating_message
    && crate::webhook_dispatch::is_unauthorized_webhook_dispatch(msg)
{ ... }

// EndTurn INTENT_GUARD (agent loop)
fn webhook_no_unauthorized_dispatch_trigger(msg: &str) -> bool {
    is_unauthorized_webhook_dispatch(msg)
}
```

A single source of truth eliminates drift by construction. Any change to the predicate's semantics updates both consumers atomically.

### Fallback: symmetric SQL with paired-clause invariant (mika#1163's approach)

When the two predicates differ in shape (e.g., one returns a row tuple, the other returns a count; one is per-class scoped, the other is agent-wide), shared-function extraction is invasive. The lighter fix is to maintain symmetric SQL clauses with a comment-level invariant + structural test:

```sql
-- Both slot predicates must include this clause (mika#1163).
-- Drift between them creates the asymmetric-perimeter deadlock class.
AND label NOT LIKE '%:deferred'
```

Pair this with a test that greps both predicate SQL strings and asserts they contain the symmetric clauses (structural parity test). The unification refactor can be deferred until a third caller appears (rule of three).

### Don't: prose-only "remember to update both" comments

A comment like "remember to update `has_any_active_callback` if you change this" is not a structural guard. It relies on the next contributor reading and acting on it — which is the failure mode that produces the drift in the first place. mika#1163's CLAUDE.md update explicitly states the symmetric-exclusion contract, but it's belt-and-suspenders on top of the actual SQL parity, not the primary defense.

## Detection

### During code review

When ANY of these reviewers flag the same code region with the same kind of finding, treat it as cross-perimeter drift signal:

- `correctness` ("logic asymmetric between sibling predicates")
- `testing` ("no test pins structural parity between two predicates")
- `adversarial` ("predicate A's exclusion set differs from predicate B's; construct a state where both fire and disagree")

mika#1163's review: three reviewers independently flagged the same gap (no structural drift guard for symmetric `:deferred` exclusion across both predicates), boosting cross-reviewer agreement to confidence 0.95. The fix added the missing test scenarios. The pattern: cross-reviewer agreement is the primary detection signal.

### In code

For each perimeter pair, ensure ONE of the following exists:

1. **Shared-function dispatch** (canonical) — both consumers import and call the same function. Drift is impossible by construction.
2. **Structural parity test** (fallback) — a test that reads both predicate SQL strings (or both predicate function bodies) and asserts they contain the symmetric clauses. Fires on drift before the next consumer hits the bug.
3. **CLAUDE.md inline contract** (last resort) — documented invariant in the closest hierarchical CLAUDE.md, with the literal SQL clause or function signature. Only a defense-in-depth layer, never the sole protection.

### After deploy

If the pair governs production-critical flow (dispatch, auth, security boundary), add a runtime observability signal that distinguishes "correctly blocked" from "asymmetric drift fired". Example for mika#1163: an alert on `grep deferred_dispatch_promoted` rate dropping to zero while pending wrappers exist would catch a future regression within minutes of deploy.

## Prevention

When you author a new structural guard, gate, or backstop in Mika:

1. **Grep for sibling predicates.** Before defining a new predicate, search for existing functions/queries that answer the same domain question. Look for: `has_active_*`, `is_*_dispatch`, `can_*`, `validate_*`. If a sibling exists, use it directly (canonical fix) — don't fork its logic.
2. **If forking is unavoidable, ship the parity test in the same commit.** A new structural perimeter without a structural parity test against its sibling is the next mika#1163.
3. **Cross-reviewer agreement is the gate.** If `correctness` + `testing` + `adversarial` all flag "no structural drift guard between predicates X and Y", do not defer the fix — that's a P3 today and a P0 incident tomorrow.
4. **Document the invariant in the closest hierarchical CLAUDE.md.** Inline contracts live in the file the predicate lives in (e.g., `crates/mika-agent/CLAUDE.md` § "Unified Task Engine" for mika#1163's pair). Top-level CLAUDE.md is too far from the code.

## Related

- mika#910 — `docs/solutions/architecture-patterns/intent-guard-predicate-sharing-2026-05-14.md` — webhook guard pair (first documented instance)
- mika#1070 + mika#1163 — `docs/solutions/logic-errors/deferred-dispatch-promotion-deadlock-2026-05-10.md` — slot guard pair (second documented instance, second drift event)
- mika#525, mika#549, mika#1058 — `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` and `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md` — active-dispatch check pair (implicit third instance, no drift yet but same structural shape)
- mika#1124 — `dispatcher.rs:2339-2351` drift guard for `DEFERRED_DISPATCH_LABEL` constant (pattern reference: structural test pinning a load-bearing convention)
