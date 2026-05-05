---
title: "Frame discovery deliverables on hand-off to a fresh grooming session"
date: 2026-05-05
category: best-practices
module: arch-handoff
problem_type: best_practice
component: development_workflow
severity: medium
tags:
  - arch-handoff
  - grooming
  - framing-header
  - peer-review
  - discovery-brief
  - ratification-vs-discovery
applies_when:
  - Sending a previously peer-reviewed analysis (discovery brief, brainstorm, option-comparison doc) to a fresh `mika-arch` grooming session
  - Re-using an upstream artifact as input to `/mika-groom-ticket`, `/mika-ask-arch`, or any architect interaction
  - Risk that the fresh session re-litigates settled material instead of advancing the ask
related_components:
  - documentation
---

# Frame discovery deliverables on hand-off to a fresh grooming session

## Context

A discovery deliverable (empirical analysis + option comparison + recommendation) arrives at architect grooming carrying its original review contract: "review my analysis." The architect's default reflex on receiving a long analysis document is to engage with it as analysis — meaning peer-review-style validation of the empirical claims and recommendations.

When the next ask is *grooming* (produce an implementation plan), that default reflex produces another peer-review pass instead of grooming output. The friction surfaces specifically when:
- The original peer-review on the discovery deliverable already happened (different session)
- The deliverable is being handed off to a *fresh* grooming session
- The actual ask has shifted from "validate this" to "build a plan against this"

Without a redirect, you get the wrong artifact back: another set of reviewer notes when you wanted a grooming plan with consistency-recovery shape picked, schema migration approach, write-site enumeration, and test plan.

## Guidance

When sending a previously peer-reviewed analysis to a fresh grooming session, prepend a 5–10 line framing header that does three things:

1. **State the actual ask explicitly.** Name the deliverable expected: "implementation plan suitable for `/mika-groom-ticket` to consume," not just "grooming plan." Be specific about the shape — consistency model, schema migration approach, write-site enumeration, test plan.

2. **Mark prior context as settled.** Cite the peer-review session ID. Say "do not re-litigate" verbatim. Anyone reading the session later sees the boundary between the analysis (settled upstream) and the grooming output (the actual deliverable). Prevents the fresh session from defaulting to peer-review framing.

3. **Name the open call.** Identify the specific design decision the architect should make — the load-bearing implementation choice that grooming should produce a *recommendation* on, not just a plan around. Without this, the architect produces something stylistically grooming-shaped but missing the load-bearing decision.

The header replaces the analysis document's *original* contract with the *new* contract. The analysis content is the input; the header is the contract.

## Why This Matters

Architects (whether `mika-arch` or any reviewer) operate on the contract embedded in the artifact they receive. A discovery deliverable carries a review contract by default — that's what discovery deliverables are for. Without redirect, the contract follows the artifact into the fresh session.

The cost of getting it wrong: a fourth peer-review pass on already-settled analysis instead of a grooming plan. That's a wasted architect cycle, an audit-trail collision (now multiple sessions reviewing the same analysis), and a delay on the actual implementation.

The cost of the framing header: 5–10 lines, ~30 seconds to write. The asymmetry is steep.

## When to Apply

- **Apply** when a peer-reviewed artifact is the input to a grooming/implementation-planning session, and the new session is fresh (no prior context with you).
- **Apply** when the artifact's structure (analysis sections, option comparison, recommendation) is identical to what the original peer-review session produced. Architect's default reflex matches the artifact's shape.
- **Skip** when you're continuing a session that already has the right contract — the existing thread carries it.
- **Skip** when the artifact was authored *for* grooming consumption from the start (e.g., a grooming brief written explicitly by the architect's prior session). The contract already matches.

## Examples

**Empirical instance (2026-05-05, `mika-arch` session `6e13e3e1-a7dc-4500-ae17-1b13f96c3488`):**

A discovery deliverable on session-continuity-across-scope-types had been peer-reviewed in `mika-arch` session `01963864-7c63-4242-a1ff-718941618f8a` two hours earlier. The deliverable contained: empirical analysis (compaction's agent-scoped DELETE, 98.5% deletion rate), three-option comparison (A/B/C), and a Vincent-decided recommendation (Option C). Sending the same deliverable raw to a fresh grooming session would have reproduced the peer-review framing.

The framing header used:

```
# Grooming request — mika#974

Asking for a **grooming plan suitable for `/mika-groom-ticket` to consume**, not a peer-review pass on the analysis below.

**Prior context (settled — do not re-litigate):**
- Empirical analysis, three-option comparison (A/B/C), and decision rationale were peer-reviewed in arch session `01963864-...`. The session validated all empirical claims and surfaced the universal untagged-row fallback constraint.
- Vincent selected **Option C** (parallel non-compacted `task_messages` table) on platform-direction grounds. Storage-trajectory data, Q1/Q2/Q3 reasoning, and the decision premise are recorded in the ticket body and the brainstorm doc.
- Cold-start universal-fallback constraint is folded into ticket §2.

**Open design call for grooming:**
- **Consistency-recovery shape on double-write** (ticket §2, options a/b/c): single SQLite transaction (a), idempotent retry (b), or reconciliation worker (c). Recommend (a) by default; grooming pass owns the final call.

**Deliverable expected:** implementation plan with consistency model picked, schema migration approach (vN→vN+1), write-site enumeration, and test plan covering BOTH happy-path replay AND mixed-tagging acceptance criterion.

---

[discovery deliverable content follows]
```

Result: arch returned a grooming plan with disposition READY, picked option (a) with rationale, produced Phase 0 pre-coding pins (six load-bearing line ranges to verify before code lands), and a 5-unit implementation plan with 6 tests. No peer-review re-litigation. Plan filed as PR #977.

**The before/after framing:**

| Without header | With header |
|---|---|
| Architect re-validates the analysis | Architect produces grooming plan |
| Returns reviewer notes | Returns implementation units + test plan + design call resolved |
| 4th peer-review pass | 1st grooming pass |
| Audit trail: multiple sessions on same analysis | Audit trail: one peer-review session, one grooming session, distinct contracts |

## Related

- Memory: `feedback_peer_review_ratification_vs_discovery.md` — calibration heuristic on when peer-review framing drifts toward "ratification before action." This pattern is the operational counterpart: how to redirect arch *away* from ratification framing when the artifact's shape would otherwise invite it.
- Adjacent pattern: `mika/docs/solutions/best-practices/quoted-resource-pre-fetch-guard-brief-content-augmentation-2026-04-29.md` (brief-content augmentation for required-tools enforcement) — shares the "header-prefix shapes downstream behavior" technique at a different layer.
- Empirical artifact: `mika/docs/brainstorms/2026-05-05-session-continuity-across-scope-types-brainstorm.md` (the discovery deliverable that produced this learning).
- Architect sessions: `01963864-7c63-4242-a1ff-718941618f8a` (peer-review on brief), `6e13e3e1-a7dc-4500-ae17-1b13f96c3488` (grooming on mika#974) — two distinct contracts on the same content.
- Implementation ticket: mika#974 (`feat(task_engine): introduce task_messages parallel narrative table`).
