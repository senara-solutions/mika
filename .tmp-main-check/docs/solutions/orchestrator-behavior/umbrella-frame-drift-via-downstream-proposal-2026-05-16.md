---
title: Umbrella frame drift via downstream proposal
module: orchestrator-claude
date: 2026-05-16
problem_type: orchestrator_behavior
component: orchestration
severity: medium
tags:
  - orchestrator-claude
  - umbrella
  - framing
  - peer-review
  - handsoff
related_components:
  - milestone-fit-audit
  - claude-friend-brief
  - handsoff-log
applies_when:
  - "A ticket has been paused under a named operator-framing umbrella"
  - "A downstream proposal (milestone-fit audit, grooming review, etc.) suggests a disposition for that ticket"
  - "The proposal does not name or re-derive the existing umbrella before proposing"
---

# Umbrella frame drift via downstream proposal

## Symptom

Tickets paused under a named operator-framing umbrella (a thematic anchor like "self-awareness research project framing" or "KG-deprecation candidate set") get **silently re-framed** by a later downstream proposal — a milestone-fit audit, a grooming pass, a closure sweep. The proposal looks coherent on its own terms: it cites code state, lists the tickets, proposes a disposition (close, reroute, retire). What it does not do is re-derive or name the active umbrella, so the umbrella's original frame is dropped without a record.

The drift is **not detection-evident at proposal time**. The proposal reads correctly against current code; nothing in the proposal itself flags "this contradicts a paused-under-umbrella decision from N days ago." The umbrella only resurfaces if a peer-reviewer or operator independently recalls it.

## Canonical instance (2026-05-16)

mika tickets #1076, #1077, #1091, #960 were paused per the 2026-05-12 operator audit (decision A: "park under the self-awareness research project framing umbrella; resume when that frame is settled").

On 2026-05-16, orchestrator-Claude ran a milestone-fit audit for milestone#24 ("KG observability & resolver hardening") and proposed:

- Attach #1076, #1077, #1091, #960 to milestone#24.
- Close them under a "KG-audit-deprecation" frame (i.e., "these are KG observability concerns the audit retired").

The proposal stood up against current KG code. It did not contradict any source-of-truth code claim. **It silently dropped the self-awareness umbrella.**

A peer-review pass (Claude-friend brief, manually triggered) flagged the drift. Reading the rolling 7-day handsoff window (2026-05-12, -13, -14, -15):

- May 12 handsoff explicitly establishes the umbrella and lists the four tickets under it.
- May 13 handsoff references the umbrella (no retirement signal).
- May 14 handsoff references the umbrella (no retirement signal).
- May 15 handsoff is silent on the umbrella (but does not retire it).

No handsoff retires the umbrella. The umbrella is alive. The proposed disposition contradicts it.

## Lesson 1 — Before disposing a paused ticket, grep the handsoff window

Recipe before any "close/reroute/attach-to-milestone" proposal for a paused ticket:

```sh
# Grep the rolling 7-day handsoff window for the ticket number and umbrella keywords
rg -i -e "<ticket-num>" -e "paused" -e "umbrella" -e "anchor" docs/logs/handsoff-*.md
```

Inspect every hit. **If any handsoff names the ticket under an umbrella and no later handsoff explicitly retires it, the umbrella is alive.** Default to alive — retirement must be positive and explicit, not inferred from silence.

A handsoff going silent on an umbrella is not retirement. It's just a handsoff that didn't have anything new to say. Retirement is a positive event ("umbrella X is retired because Y was settled") and must be cited as such.

## Lesson 2 — Park-vs-close reversibility asymmetry

A closed ticket with body text "reopen if X" is functionally similar to a parked ticket with the same trigger — but **discovery cost differs sharply**:

| State | GitHub `gh issue list` default | `gh search issues` default | Operator-memory recall |
|-------|--------------------------------|----------------------------|------------------------|
| Open + label `paused` | Found | Found | Easier (issue is visible) |
| Closed + body `reopen if X` | **Missed** (defaults exclude closed) | **Missed** (defaults to is:open) | Harder (out of view) |

When the un-pause trigger is a framing decision with no natural deadline (e.g., "resume when the self-awareness research frame is settled"), park is strictly reversible — re-open is a label flip. Close is functionally reversible but **discoverability-asymmetric**: closed tickets fade from default views, so the trigger-event-when-it-arrives is less likely to find them.

**Default to park when the un-pause trigger is a framing decision.** Reserve close for tickets where the un-pause condition is impossible or the work is genuinely retired.

## Lesson 3 — Framing-trigger umbrellas have no natural deadline

An umbrella whose un-pause condition is "the framing decision is settled" has no clock. It can quietly become permanent — the parked tickets accumulate cruft, the framing is never revisited, and a future audit (like the one above) sees a coherent set of tickets that "look stale" and proposes a retirement.

The trigger gates un-pause, not "has anyone attempted the framing decision in the last N weeks." So nothing in the system naturally surfaces "is this umbrella stuck?"

**Mitigation:** surface a periodic check-in question separate from the un-pause question, e.g., "umbrella X has been open for 30 days, are we still planning to settle the framing?" The answer can be "yes, keep parked" — that's a positive signal that the umbrella is alive, not a re-derivation of the original decision. Without this check-in, framing-trigger umbrellas drift into accidental permanence.

## How this differs from other drift modes

| Drift mode | Frame anchor | Detection path |
|------------|--------------|----------------|
| Body callout drift (Class C) | Issue body callout content | Mechanical (grep ticket number against callout path) |
| Plan-doc rebase identity (mika-arch) | Plan blob hash vs ancestry | Mechanical (blob hash comparison) |
| **Umbrella frame drift** | **Operator's handsoff log** | **Manual handsoff grep + memory recall** |

Umbrella drift has no mechanical anchor in code, schema, or issue metadata. The umbrella exists in operator narrative and orchestrator memory. The detection cost is therefore higher — the system cannot detect it for you; you must remember to grep.

## Related

- Memory: `feedback_dont_drift_umbrella_frame` (filed 2026-05-16).
- Handsoff sequence: docs/logs/handsoff-2026-05-12.md (umbrella established), -05-13, -05-14, -05-15 (silent but not retiring).
- Adjacent: `compound-engineering:ce-review` peer-brief pattern as the catch path when no mechanical anchor exists.
