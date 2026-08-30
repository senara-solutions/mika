---
title: "No production substrate on an open, documented failure mode"
module: loop-substrate
date: 2026-08-30
problem_type: best_practice
component: dispatch
severity: high
tags: [failure-mode, substrate, risk, open-ticket, mitigation, model-swap, empty-output, doctrine]
applies_when: "A known failure mode has an open ticket and something in production continues to run on the component it describes"
---

# No production substrate on an open, documented failure mode

## Context

mika#1910 — glm-5.2 returning a silent empty output at `max_turns` — was **open and documented** while the entire autonomous loop ran on that component. The ticket named the exact way a session could end having produced nothing while reporting success. Nobody had to discover the failure mode; it was written down.

The empirical result, measured on 2026-08-26 (13:35Z): eight active pilots, three pushed branches, **zero PRs opened in over two hours**. The work was recovered by nets — post-flight recovery (#1282) and auto-PR-create rescue (#1383) — which is to say it was recovered by the parts of the system built for when the nominal path fails, running as the nominal path.

Two months later the shape had not changed: on 2026-08-29, 102 of the 120 most recent sessions made zero tool calls.

## Guidance

**When a failure-mode ticket opens, the substrates that exercise it are part of the ticket.**

An open failure mode is not a note about the future. It is a statement that a component in use can fail in a named way, right now. Running a production substrate on it without an active mitigation is choosing that failure and calling it a surprise when it arrives.

The operational corollary, in order:

1. **Name the substrates.** When a failure-mode ticket opens, list what in production exercises the component. If that list cannot be produced, that is the first finding.
2. **Switch, gate, or accept in writing.** Either move the substrate off the component, put a human gate in front of it, or record the acceptance with a reason and a review date. Silence is none of the three.
3. **A mitigation is only active once measured.** A model swap, a config flip, a version pin — all of them are *claims* until a measurement taken after the change shows the failure mode gone. mika-dev was swapped from glm-5.2 to glm-5.3 on 2026-08-26; the 2026-08-29 measurement still found 102 of 120 sessions silent. Closing #1910 on the strength of the swap would have recorded a repair that the next measurement contradicts.
4. **Do not close a failure-mode ticket on the strength of a mitigation.** The mitigation belongs in the ticket as a status note. Closure needs either a fixed root cause or a fresh measurement showing the class is gone — the same standard applied to any other claim.

**Why this is worth a rule rather than judgment.** A documented-but-open failure mode is the cheapest possible warning: someone already did the diagnostic work. Building on it anyway converts a known risk into an incident, and then spends the incident budget re-learning what the ticket said. The nets that catch the fallout make the arrangement feel survivable, which is precisely what lets it persist — a rescue net running as the nominal path is not resilience, it is an outage with good manners.

## Reference

- Founding incident: mika#1910 (open + documented while the loop ran on it), operator report 2026-08-26 13:35Z
- Detection that closes the silent half: mika#1996, [`cycle-non-empty-detector-2026-08-30.md`](cycle-non-empty-detector-2026-08-30.md)
- Related: mika#1901 (residual stall), mika#1282 / mika#1383 (the nets), mika#1991 (`optional-path-is-no-guarantee`)
