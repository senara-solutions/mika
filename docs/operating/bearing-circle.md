# Bearing circle — who talks to Mika Prime (mika#1641 AC4)

> **Status: DECISION PENDING (Vincent-only).** This document records the
> bearing-circle question the orchestrator role transfer (mika#1641) raises, the
> A/B/C options, and the gate contract. It **surfaces** the decision; it does not
> resolve it. Per the hard rule `feedback-prime-conversation-circle-closed`
> (2026-06-19), the architect cannot ratify a change to a closed conversation
> circle — only Vincent can. When Vincent picks an option, record it in the
> "Decision" section below and update `feedback-prime-conversation-circle-closed`
> if the answer expands the circle, so future operators do not relitigate.

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

> _Unfilled. Vincent has not yet picked A/B/C. Fill this section with the chosen
> option, the date, and a one-line rationale when the call is made. Until then, the
> orchestrator (Mika) does **not** assume direct Prime access — she routes any
> Prime-level call through Vincent (the conservative default, equivalent to option C)
> pending the decision._

- **Chosen option:** _pending_
- **Decided by:** _pending (Vincent)_
- **Date:** _pending_
- **Rationale:** _pending_
