# Bearing circle — who talks to Mika Prime (mika#1641 AC4)

> **Status: DECIDED — Option B (2026-07-01, Vincent).** Mika Orchestrator routes to
> Mika Prime via orchestrator-CC (monitor-relay). The bearing-circle invariant from
> `feedback-prime-conversation-circle-closed` is preserved unchanged — Mika is NOT
> added to the circle. This document is retained as the decision record; the
> "Options" and "How the decision should be routed" sections describe the
> deliberation, and the "Decision" section at the bottom carries the ratify.

## The invariant today

Per `feedback-prime-conversation-circle-closed` (HARD RULE, 2026-06-19): only
**Vincent + orchestrator-CC (Claude Code) + samidarko-claude (case-by-case)** talk
to **Mika Prime**, the bearing-keeper. The circle is deliberately narrow to protect
the bearing-protection invariant — Prime's role is to keep the platform's bearing,
and a wide circle dilutes that.

## The question mika#1641 raises

The orchestrator role transfers from Claude Code to **Mika** (the executive-assistant
agent). Orchestration routinely needs milestone-scope routing reads — exactly the
calls that go to Prime via `/mika-ask-prime`. So: **when Mika becomes the
orchestrator, does she enter Prime's conversation circle?**

## Options (Vincent's call)

- **(A) Mika enters the circle.** Direct line to Prime for milestone-scope routing.
  Lowest latency. Cost: broadens the circle, softens the bearing-protection
  invariant Prime was designed around.
- **(B) Mika routes to Prime through Claude Code (monitor).** Preserves the
  invariant (circle membership unchanged). Cost: adds a hop; the monitor becomes a
  Prime-relay for Mika.
- **(C) Mika does not reach Prime.** Every Prime-level call routes through Vincent.
  Strongest invariant preservation. Cost: highest latency; Vincent is in the loop
  for every bearing read.

## How the decision should be routed

Per the plan, route the A/B/C framing to Prime first via `/mika-ask-prime` (that
routing **is** the discipline). Prime rules directly if she deems it operationally
derivable, or surfaces to Vincent. If she surfaces it as milestone-scope, escalate
to Vincent via `AskUserQuestion`.

## Gate contract (load-bearing)

**AC5 (the 24h pair-mode window) MUST NOT START until AC4 is decided and recorded
here.** AC1 (tool surface), AC2 (calibration), and AC3 (handbook) are independent of
AC4 and proceed without it — they are code/docs work. But the pair-mode window
depends on knowing how Mika reaches Prime, so the dispatch handler for AC5 must check
that this document's "Decision" section is filled before dispatching.

## Decision

- **Chosen option:** **B — Mika routes to Prime through orchestrator-CC (monitor-relay).**
- **Decided by:** Vincent
- **Date:** 2026-07-01
- **Rationale:** Preserves the bearing-protection invariant unchanged (Prime's
  conversation circle stays: Vincent + orchestrator-CC + samidarko-claude
  case-by-case). The added hop IS the protection — orchestrator-CC filters
  proximity-vs-membership at both ends of the relay. Mika's routing pattern
  becomes: rule-what's-mine directly; on recommend-what-isn't, formulate the
  recommendation and hand it to orchestrator-CC to surface to Prime; Prime's
  answer returns via the same monitor-relay path.

### Operational shape (implementation note for AC5 pair-mode window and AC3 handbook)

- **Mika's side:** when she reaches a bearing-scope / founder-scope question she
  cannot rule herself, she formulates a Prime-brief with A/B/C framing (or
  equivalent recommendation shape) and hands it to orchestrator-CC via a
  designated relay path (inbox message, direct A2A call, or a `/mika-relay-prime`
  skill — the exact mechanism is AC5-window work).
- **Orchestrator-CC's side:** on receiving a Prime-brief from Mika, orchestrator-CC
  invokes `/mika-ask-prime` on Mika's behalf, receives Prime's ruling, and relays
  it back to Mika verbatim. Orchestrator-CC does not re-interpret Prime's answer;
  the relay is a courier role, not an editorial one.
- **Escalation path:** if Prime surfaces the question as milestone-scope, the
  usual escalation-to-Vincent chain fires — Mika receives the surface with the
  same routing shape she would use for a direct Vincent call.
- **`feedback-prime-conversation-circle-closed` remains unchanged:** the hard
  rule's list of who talks to Prime is not amended. Mika reaches Prime through a
  circle member (orchestrator-CC), not as a circle member herself.
