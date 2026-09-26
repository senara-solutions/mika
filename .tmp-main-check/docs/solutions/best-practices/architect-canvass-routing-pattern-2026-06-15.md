---
module: orchestration, mika-arch, mika-prime
tags: [orchestration, canvass, routing, mika-arch, mika-prime, operator-territory, decision-routing]
problem_type: best-practice
category: best-practices
date: 2026-06-15
ticket: mika#1538 (founding incident)
applies_when:
  - Running an architect canvass on a ticket that needs operator scope decisions
  - "Architect returns `Disposition: ESCALATE` with blocking findings"
  - Operator gives a terse reply ("ask her", "do it", "ok") that could be read as full delegation or as routing-pointer
  - Distinguishing operationally-derivable calls from operator-territory calls under ambiguity
resolution_type: discipline
---

# Architect canvass routing pattern — Vincent ↔ Prime ↔ orchestrator-CC

## TL;DR

When mika-arch's canvass surfaces blocking findings that touch operator-territory (substrate-anchoring, milestone-scope-shape), the routing is **architect → Prime → operator**, not architect → operator directly. Mika Prime triages whether each finding is operationally-derivable (she rules) or operator-territory (she frames for operator). When the operator's reply is terse ("ask her"), **read it as routing-pointer, not authority-grant** — bearing-keeper bias on ambiguous warrants is bring-it-back-framed, not forward-into-ruling.

## Founding incident (2026-06-14/15, mika#1538)

Orchestrator-CC ran an architect canvass on mika#1538 (ascension slice 2). mika-arch returned `Disposition: ESCALATE` with three blockers:

- **F1** — R-number drift between brainstorm and slice 1 plan (mechanical doc fix)
- **F2** — Signing-key locus choice ((a) gateway / (b) pod / (c) operator-bundle); architect recommended (c)
- **F3** — R4+R7 coupling: defer both to slice 3 vs ship R4 partial; architect recommended defer

Orchestrator-CC routed to Mika Prime per `feedback_orchestrator_questions_route_through_prime`. Prime's ruling:

- **F1 → inline** (mechanical, doctrine names brainstorm as origin-of-truth, no operator-territory binding)
- **F2 → surface to Vincent** (architect recommendation doesn't transfer ownership of "whose key is the security root")
- **F3 → surface to Vincent** (slice-scope mutation is operator-territory by structural placement, even when doctrine implies the call)

Operator's reply to the framed F2+F3 surface: **"ask her"** (two words, no qualifiers).

Orchestrator-CC's first read: full delegation. Wrote a bearing brief asking Prime to substantively rule.

**Prime refused the delegation read.** Her response:

> Two words, no qualifiers, no rationale. That's not how Vincent delegates substantive scope-calls when he's actually delegating them. When he wants me to rule, he names what he's ruling on — "Prime, you call it," "your decision," "I'll ratify whatever you pick." "Ask her" is a different shape: it's *routing-pointer*, not *authority-grant*.

Prime gave substantive recommendations (not rulings) and re-framed for operator ratification. Operator ratified both: "ratify both."

## The pattern

### Stage 1 — Canvass

Orchestrator-CC composes a focused brief and invokes `mika-ask-arch` (with file-body `@/tmp/canvass-NNNN.md`). The brief asks for:
1. Scope verdict on the ticket's stated decomposition
2. Which open questions must resolve before grooming starts
3. Implementation-unit sequencing risks
4. Largest planning risks
5. `Disposition: READY | ITERATE | ESCALATE`

Architect returns prose findings + a disposition line. Findings flagged `(BLOCKING)` indicate they prevent grooming.

### Stage 2 — Triage via Mika Prime

If architect returns `ESCALATE` with blockers, orchestrator-CC does NOT surface directly to operator. It composes a routing brief for Mika Prime via `mika-ask-prime` listing:

- Each blocker with architect's recommendation
- Orchestrator-CC's read of whether each is operationally-derivable or operator-territory
- The bigger frame: surface the whole canvass, or record-and-proceed?

Prime triages each blocker into:
- **Operationally-derivable** → orchestrator-CC resolves inline (mechanical doc fixes, doctrine-named calls without operator-territory bindings)
- **Operator-territory** → Prime authorizes orchestrator-CC to surface to operator with a specific framed shape

Prime's discriminators for operator-territory:
- **Substrate-anchoring decisions** — *whose key, whose identity, whose name* gets bound as a security root or canonical reference. Architect recommendation doesn't transfer asset ownership.
- **Milestone-scope mutations** — adding or removing work from a named milestone slice. Bearing-keeper informs scope; operator rules it.
- **Road-shape decisions** — sequencing of slices on a stated trajectory. The trajectory is operator's road.

Prime's discriminators for operationally-derivable:
- **Doctrine-derived calls** — when established memory or documentation names the source-of-truth or the rule. Bearing-keeper can apply doctrine.
- **Mechanical reconciliation** — drift between documents where one is canonical and the other drifted.
- **Recording non-blocking findings** — architect-output worth preserving on a ticket regardless of how the scope decisions land.

### Stage 3 — Surface to operator (only with Prime authorization)

When Prime authorizes a surface, she typically provides the exact framed text to bring to operator. Orchestrator-CC surfaces this verbatim (or near-verbatim), structured as:

- The decision needed (named explicitly)
- The options with their bindings ("what does this commit?")
- Architect's recommendation with rationale
- Operator: ratify or override

### Stage 4 — Handle operator reply

Operator replies are sometimes terse. The **terseness is signal, not warrant**:

- **Full sentence delegation** ("Prime, you call it" / "your decision" / "I'll ratify whatever you pick") → Prime takes the ruling
- **Terse routing pointer** ("ask her" / "go ahead" / "ok") → ambiguous, bias to bring-it-back-framed
- **Direct ratification or override** ("ratify both" / "go with (a) instead") → orchestrator-CC records and proceeds

When the reply is ambiguous, orchestrator-CC re-invokes Prime via the same session, asking for substantive recommendations (NOT rulings), and the cycle returns to Stage 3.

### Stage 5 — Record + dispatch

Once operator ratifies, orchestrator-CC records the decisions on the ticket (comment with full trail), fixes any reconciliation docs (PR), then either:
- Labels `ready` for full autonomous-loop dispatch (small/contained tickets)
- Spawns `/mika-spawn /mika-groom-ticket NNNN` for manual groom-only dispatch (large/architectural tickets where operator wants to review the plan before dev-pilot fires)

## Anti-patterns

### The "destination-right" carve-out

> "If you'd have chosen (c) anyway, I can just record it on #1538 and proceed."

This is the carve-out shape Prime explicitly rejected in the founding incident. It compresses operator-territory by saying "I'll take the call IF it's what you'd have chosen anyway" — but the test of whether it's what they'd have chosen requires asking, which negates the carve-out.

Doctrine doesn't have a "destination-right" or "framing-economical" carve-out for substrate-anchoring or milestone-scope decisions. Even with architect's recommendation in hand, even with high confidence in the answer, the binding belongs to the operator.

### Doctrine-implies vs doctrine-derives

> "Vincent has consistently held coherence over partial features, so deferring R4 is operationally-derivable."

When doctrine *implies* a call that also happens to mutate operator-territory, the implication informs the surface — it doesn't substitute for it. The discipline holds the operator-bearing-keeper-hand layering clean by surfacing implications, not taking them as derivations.

### Terseness as license

> "Vincent said 'ask her' — that's full delegation."

Two-word replies are signals, not warrants. When the authority-signal is ambiguous, bearing-keeper bias is *back to operator*, not *forward into ruling*. The cost of getting the warrant-read wrong in the take-the-call direction (substrate-binding on operator's behalf without explicit warrant) is much higher than the cost of getting it wrong in the bring-it-back-framed direction (one round-trip).

## Related

- `feedback_orchestrator_questions_route_through_prime` — the structural rule (questions to operator route through Prime first)
- `feedback_operator_is_escalate_only` — Prime is the routine-decision layer; operator is escalate-only
- `feedback_samidarko_claude_new_center_keep_pushing` — samidarko-claude is the new center for routine routing
- `mika#1244` — Unresolved-Decision Gate (plans with TBD decisions cannot reach READY)
- `mika#1538` — founding incident (ascension slice 2 canvass + ratification)
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — architect disposition keyword discipline

## Out of scope

- Direct mika-arch ← → operator routing without Prime in the middle. Architect dispositions are technical findings; routing them to operator requires bearing-keeper triage by design.
- Skipping the canvass entirely when a ticket "feels groomable" — the canvass is the discriminator between "operator scopes before grooming" and "autonomous-dispatch safe." Without it, large architectural tickets routinely hit `Disposition: ESCALATE` deep in the pipeline instead of at the front gate.
