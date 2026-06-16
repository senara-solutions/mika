---
module: orchestration, decision-routing
tags: [doctrine, n-equals-1, n-equals-2, deferral, pragmatism, scope-decisions, operator-territory]
problem_type: best-practice
category: best-practices
date: 2026-06-16
ticket: control-monitor (founding case)
applies_when:
  - "Facing a doctrine-shaped question (A vs B vs C, broad rule, general policy) triggered by exactly one concrete instance"
  - "The instance can be solved pragmatically without committing to one of the broad shapes"
  - "Committing to a doctrine on n=1 would constrain decisions on hypothetical future cases that may never materialize"
resolution_type: discipline
---

# Doctrine-defer-via-move — when n=1 isn't enough data to commit to substrate-shape doctrine

## TL;DR

When mika-arch or Mika Prime reframes a single-case problem into a doctrinal question ("should mika agents work on any operator-authorized repo, or stay senara-only?"), and the case is genuinely n=1 in the data, the right answer is often **pragmatic-fix-this-one-case-and-defer-the-doctrine**, not "pick a shape from the doctrine option-list." Apply `feedback_n_equals_2_is_the_signal` at the doctrine layer: wait for a second instance before committing to a broad rule.

## Founding case (2026-06-15)

**Problem:** mika-qa structurally deadlocked on `samidarko/control-monitor` PR #14 — three locks fired (token org-scope, missing local worktree, hardcoded `build_mika`). Vincent paused control-monitor until the loop worked end-to-end on out-of-org repos.

**Mika Prime's reframe** (via `/mika-ask-prime`): the triad is three different doctrine-objects (only the token is identity-bearing). The doctrinal question isn't binary — three shapes:
- **(A) Senara-only-by-design.** mika agents stay in senara lane. Out-of-org work mirrors in or doesn't happen.
- **(B) Operator-portable.** Multi-token substrate; mika agents act on any operator-authorized repo. Real substrate project (~weeks).
- **(C) Senara-anchored + ungrounded-engagement-as-doctrine.** mika agents stay grounded in senara; explicitly authorized to engage out-of-org in ungrounded-review mode with `ungrounded=true` verdict marker.

Prime's lean: (C). Her caveat: *"this is n=1 (control-monitor, six days old). Picking (B) on n=1 is a heroic-lunge to enable a single use-case; picking (A) or (C) is the steady-beat shape."*

**Vincent's resolution:** moved the repo from `samidarko/control-monitor` to `senara-solutions/control-monitor`. Solves the specific case (auth scope realigned, worktree path can be added, build still doesn't apply but cm has own CI). Doesn't commit to any of (A)/(B)/(C) doctrine. n=2 hasn't arrived; doctrine call deferred.

## The discipline

When a single-instance problem triggers a doctrine-shaped question:

1. **Solve the case pragmatically.** Find the smallest concrete action that resolves the case without committing to a broad rule. In the founding case: move the repo.
2. **Name the deferral explicitly.** Don't pretend the doctrine question is answered. Document that the pragmatic fix sidesteps the broader question pending more data.
3. **Wait for n=2 to commit to doctrine.** Per `feedback_n_equals_2_is_the_signal`: structural decisions need the second occurrence. The first occurrence is data; the second occurrence is pattern. Committing to doctrine on the first occurrence over-constrains future judgment.
4. **Set the n=2 trigger explicitly.** Name what counts as the second occurrence. In the founding case: if/when a second user-scope project emerges that genuinely should NOT live in senara-solutions, the (B) vs (C) decision returns with real evidence.

## When this discipline applies

- A specific operational problem (n=1) gets a doctrine-shaped solution proposed
- The doctrine choice carries substrate-anchoring or scope-shape consequences (operator-territory)
- A pragmatic per-case fix exists that doesn't depend on the doctrine being picked
- The doctrine option-list contains "heroic-lunge" shapes (months of substrate work, broad authorization changes)

## When this discipline does NOT apply

- The case is genuinely the first of many (the n=2/3/4 trigger is already visible on the horizon)
- The pragmatic fix is more costly than committing to doctrine (e.g., per-case workarounds compound)
- The doctrine choice has reversibility (cheap to switch later if the chosen shape proves wrong)
- The operator explicitly asks for doctrine rather than pragmatism

## Why Prime's reframe matters even when the operator picks pragmatism

The reframe surfaces options the operator might not have considered. Even when Vincent chose "move the repo" (sidesteps all of A/B/C), Prime's three-shape framing was load-bearing because:

- It named the underlying question (mika's identity-scope) explicitly
- It established the deferral as a deliberate choice, not an oversight
- It set up the n=2 future trigger with the right vocabulary

The pragmatic answer benefits from the doctrinal framing being on the record. The doctrinal framing benefits from not being prematurely committed. Both stand.

## Anti-pattern this guards against

### Heroic-lunge doctrine commitment on n=1

> "We need a multi-token substrate so mika agents can act on any authorized repo." (on n=1)

Justification often sounds reasonable: "We'll need it eventually, might as well build it now." Real cost: weeks of substrate work designed against speculation, with the resulting design constraining future cases that haven't shown up to inform the shape.

### Avoidance-by-pragmatism without naming the deferral

> "I just moved the repo. Done."

Without explicit naming of the deferred doctrine question, the next instance triggers re-investigation from scratch. The cost of "doctrine deferred to n=2" is one line in the memory + one in the handsoff log. Skipping that line means the next instance pays the full investigation cost again.

## Cross-references

- `feedback_n_equals_2_is_the_signal` — the parent discipline for waiting on second occurrence before structural decisions
- `feedback_orchestrator_questions_route_through_prime` — the canvass routing pattern that surfaces doctrine questions in the first place
- `feedback_terse_operator_reply_is_routing_not_warrant` — Prime's distinction between substantive ruling and routing-pointer (companion discipline)
- `docs/solutions/best-practices/architect-canvass-routing-pattern-2026-06-15.md` — full architect-canvass routing pattern; this learning is the pragmatic-deferral companion

## Out of scope

- Doctrine questions that emerge from operator-stated strategic direction (not from a single case triggering a question). When Vincent says "mika should work on any operator-authorized repo," that's operator-direction, not n=1 inference — execute it, don't defer it.
- Doctrine questions where the pragmatic fix is more expensive than committing. The discipline applies when pragmatism is cheap; it doesn't apply when pragmatism IS the heroic lunge.
