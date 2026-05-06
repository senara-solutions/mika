---
module: packages/ui
tags: [design-system, primitives, ARIA, cost, reuse]
problem_type: decision
category: best-practices
date: 2026-05-06
ticket: mika#667
---

# When to add a new packages/ui primitive vs reuse an existing one

## Problem

CostMeter and TokenBudgetBar look visually similar (colored indicator + value), leading to the question: should CostMeter wrap TokenBudgetBar or be a new primitive?

## Decision

New primitive. Domain semantics drive the primitive boundary, not visual similarity.

## Reasoning

Three pinned reasons from the mika#667 plan:

1. **ARIA semantics mismatch.** TokenBudgetBar uses `role="meter"` which requires `aria-valuemax` — a known maximum. Cost has no domain maximum ($0.50, $5, $200 are all valid runs). CostMeter uses `role="status"` — unbounded, threshold-based.

2. **Threshold semantics mismatch.** TokenBudgetBar thresholds are **ratios** of value/max (0.0-1.0). CostMeter thresholds are **absolute USD amounts** ($5 warning, $20 critical). Wrapping one in the other forces consumers to compute synthetic ratios — surface-level reuse of a deeply mismatched contract.

3. **Behavior mismatch.** TokenBudgetBar clamps display at 100% (there's a max to be 100% of). Cost has no "100% full" state — clamping would lie about the displayed value.

## Pattern

The `packages/ui/CLAUDE.md` enforcement rules say: "hand-rolled implementations of these primitives are review fails." The counterbalance: **force-fitting an existing primitive on a domain mismatch is also a review fail.** The decision gate is:

| Signal | Reuse existing | New primitive |
|--------|---------------|---------------|
| Same ARIA role | ✓ | |
| Same threshold semantics | ✓ | |
| Same value domain (bounded vs unbounded) | ✓ | |
| Only visual similarity | | ✓ |
| Different evolution trajectory | | ✓ |
| 3+ consumers in the PR | | ✓ (meets promotion threshold) |

CostMeter had four consumers (Dev Run detail, LLM Calls list, Agent detail, landing widget) — well above the promotion threshold for a new primitive.
