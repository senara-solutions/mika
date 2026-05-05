---
title: "Manual fallback with gap-flagging — convert one-shot fallbacks into durable work"
date: 2026-05-05
category: best-practices
module: ticket-hygiene
problem_type: workflow_issue
component: development_workflow
severity: high
tags:
  - manual-fallback
  - gap-flagging
  - ticket-fan-out
  - orthogonal-artifacts
  - silent-gaps
  - cross-reference
applies_when:
  - A designed-but-unshipped tool, skill, or command would have produced the artifact you need now
  - A multi-component plan from a prior session has incomplete ticket coverage and a session is hitting the gap
  - Closing a session where the closure ritual itself is the thing being designed (meta-tooling work)
related_components:
  - documentation
  - tooling
---

# Manual fallback with gap-flagging — convert one-shot fallbacks into durable work

## Context

A planning thread produces an N-component plan: spec + tooling + adjacent skills, distributed across actor surfaces (operator-side, autonomous, shared contract). The thread closes with intent to file all N components. But filing happens piecemeal — typically only the most visible or highest-pressure component lands as a ticket. The other N-1 sit dormant.

Days or weeks later, a session needs the artifact one of the unshipped components would have produced. The tool isn't there. The friction surfaces. Without a discipline for what to do next, three failure modes recur:

1. **Improvise silently.** Produce the artifact in some ad-hoc format. The improvisation accumulates into tribal knowledge ("we always did it this way"); the gap closes silently and the tooling never ships.
2. **Defer.** Skip the artifact this session, plan to address tooling next session. The next session has different priorities; the gap deepens.
3. **File the missing components but leave the artifact unfiled.** Tickets go in but the session that surfaced the friction produces nothing durable that consumers (operator, next-session-Claude, autonomous loop) can read.

This pattern names a fourth mode that converges all three concerns: produce the artifact manually, file the gap, cross-reference both directions.

## Guidance

When a designed-but-unshipped tool would have produced an artifact you need now, run a three-step convergence rather than improvising:

1. **Fall back to manual** — produce the artifact by hand, in the same format and location the unshipped tool would have used. So when the tool ships, the artifact is migration-ready (same format the tool emits → no special-case handling for "manual-era" content).

2. **File the gap as ticket(s) immediately** — every missing component from the original plan becomes a ticket *this session*, with explicit `blocked-on` linkage between dependent components. Don't defer; the gap is most legible while the friction is fresh and the prior thread's intent is reconstructable.

3. **Cross-reference: both directions** — the manual artifact must cite the tickets that would have automated it (forward-pointer); the new tickets reference the format the manual artifact used (back-pointer). Future operators reading the artifact see both the content and the in-flight closure of the gap.

The convergence is the load-bearing piece. Each step alone is incomplete: manual without tickets is improvisation; tickets without artifact is defer-and-hope; manual + tickets without cross-reference produces orphaned work the next reader can't trace.

## Why This Matters

Manual fallbacks left undocumented decay into tribal knowledge. The shape of the gap closes silently — what looked like a one-time improvisation becomes "the way it's done." The original tool stays unshipped because the friction that motivated it never resurfaces sharply enough to drive ticket-filing.

Filing the gap during the fallback session converts session-local friction into a durable backlog item, with the manual artifact as evidence-of-need attached to it. Cross-referencing the artifact ↔ tickets gives the next operator a one-hop path from "I see this manual format" to "here's the ticket that automates it" — preventing the silent-tribal-knowledge failure mode.

The technique also protects against a second failure: filing N tickets but losing track of which depends on which. Explicit `blocked-on` linkage between siblings (when a contract spec must precede the tool that consumes it) keeps the work order legible to anyone groomign the ticket family.

## When to Apply

- **Apply** when a planned-but-unshipped tool/skill/command would have produced the artifact you need now.
- **Apply** when a multi-component plan from a prior session has incomplete ticket coverage and a session is hitting the gap.
- **Apply** when closing a session where the closure ritual itself is the thing being designed (meta-tooling). Self-consistency requires producing the artifact even when the producer isn't built yet.
- **Skip** when the missing tool is genuinely speculative (no prior plan exists). File a fresh ticket through the normal pipeline — there's no gap to close, just new work to scope.
- **Skip** when the artifact is one-shot and won't recur (e.g., an emergency one-off). Filing tickets for a tool you'll never need again is overhead.

## Examples

**Empirical instance (2026-05-05, session `c1c474a5-185c-4d29-93b7-4a96be1ec0d3`):**

A prior thread (referenced as 67d85cfa, dated 2026-05-04) produced a three-component handsoff infrastructure plan:

| # | Component | Actor | Task |
|---|---|---|---|
| 1 | `HANDSOFF-CONTRACT.md` spec | shared (both consumers cite it) | Format spec |
| 2 | `/mika-handsoff` slash command | operator-Claude | Conversational mode wrap-up tool |
| 3 | `dev-handsoff` bundled skill | autonomous mika-dev | Subprocess artifact writer |

At original plan time, only Task 3 was filed (mika#967). Tasks 1 and 2 sat dormant.

The 2026-05-05 session needed an end-of-session handsoff. `/mika-handsoff` (Task 2) didn't exist. `HANDSOFF-CONTRACT.md` (Task 1) didn't exist either, so even writing the manual artifact had no spec to reference.

The three-step convergence executed:

1. **Manual fallback artifact:** wrote `mika-platform/docs/logs/2026-05-05 - dashboard milestone dispatched + task_messages design + handsoff protocol gap closed.md` by hand, in the same format `/mika-handsoff` would have emitted (TL;DR / Story so far / Tickets touched / Blocked / Next-session table). Migration-ready when Task 2 ships.

2. **Gap-tickets filed same session:** **mika-platform#80** (HANDSOFF-CONTRACT spec) and **mika-platform#81** (`/mika-handsoff` slash command, `blocked-on: mika-platform#80`). Existing **mika#967** updated with a "Spec dependency" header pointing at mika-platform#80 (parallel consumer alignment). Three-ticket family with explicit dependency edges visible.

3. **Cross-references both directions:**
   - Manual log's TL;DR cites "mika-platform#80 + mika-platform#81" by number.
   - Manual log has a "Process gaps surfaced this session" section: *"67d85cfa thread produced three-task plan; only Task 3 was filed at original plan time. Tasks 1 and 2 filed today as #80 + #81. This log written manually as fallback pending #81 ship."*
   - mika-platform#80's body references the existing handsoff log convention (codifies what the manual artifact instantiates).
   - mika-platform#81's body cites mika-platform#80 by path: "the slash command's body cites the contract by path" — DRY at the spec layer.

**The before/after:**

| Without the convergence | With the convergence |
|---|---|
| Manual artifact written, no tickets filed | Manual artifact + 2 tickets, 3-ticket family with edges |
| Original plan stays half-shipped indefinitely | Gap is durable backlog with evidence-of-need attached |
| Next operator can't trace the format | Forward-pointer from artifact to tickets; back-pointer in tickets |
| Tribal knowledge accumulates | Spec converges; manual era ends when Task 2 ships |

## Related

- **Compounded peer patterns** — the bounded-fallback / operator-replicates-autonomous patterns this synthesizes:
  - `mika/docs/solutions/best-practices/bounded-b-fallback-operator-cadence-enforced-2026-05-02.md` — bounded-B fallback discipline (don't loop forever; switch to manual at threshold).
  - `mika/docs/solutions/best-practices/operator-grooming-marathon-2026-05-02.md` — operator-Claude replicates dev-groom autonomous path with skill-disable bypass; describes the manual-substitute half of this pattern.
  - `mika/docs/solutions/best-practices/f8c-sibling-loophole-pass-quality-vs-pass-count-2026-05-02.md` — operator-authored architect-shaped review when skill defects block.

  This pattern is the **convergence**: those three describe components (when to fall back, how to substitute, how to author manually); this one names the discipline of also filing the gap and cross-referencing back.

- **Adjacent memory:** `feedback_secondary_pr_plan_doc.md` — same shape at a different layer (commit plan doc for secondary cross-repo PRs *before* first push, so artifact is citable when push lands). "File the thing that makes the other thing citable" generalizes.
- **Push gate that motivates the cross-reference:** `feedback_coordination_branch_on_origin.md` — handsoff docs left local-only fail their reconciliation purpose. The cross-reference half of this pattern keeps ticket-filing aligned with that gate's reasoning.
- **Three-ticket family** filed via this pattern: mika-platform#80 + mika-platform#81 + mika#967.
- **Originating thread:** 67d85cfa (2026-05-04) produced the three-component plan; this session closed the N-1 gaps it left.
