---
title: "dev-groom post-flight recovery captures plan + branch but architect second-pass verdict never lands in body callout"
date: 2026-05-21
category: agent-quality
module: dev-groom
problem_type: behavior_drift
component: dev-groom
symptoms:
  - "Body callout has 'Branch:' + 'Plan:' lines but Grooming history reads: 'body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required'"
  - "Supervisor task transitions to 'blocked' with empty `result` after pilot exits Success"
  - "Plan file exists on branch (`docs/plans/YYYY-MM-DD-*-plan.md` committed)"
  - "Branch is pushed to origin"
  - "NO second-pass `(GROOMED)` literal in body — `dispatch_no_grooming_marker` gate will reject on subsequent ready-label dispatch"
root_cause: behavior_drift
resolution_type: investigation_needed
severity: high
tags:
  - dev-groom
  - post-flight
  - architect-verdict
  - mika#1123
  - grooming-marker
related:
  - mika#965
  - mika#974
  - mika#1172
  - mika#1224
  - mika#1123
---

## Symptoms

Operator dispatches dev-groom via `mika ask --agent mika-dev "groom mika issue#N"`. Pilot spawns, runs 18-50 turns ($0.50-$2), exits with `Success`. Mika-dev's callback handler transitions the supervisor task to `blocked`. Body callout examined:

```
> - **Branch:** `<feature-branch>` 
> - **Plan:** `mika/docs/plans/YYYY-MM-DD-*-plan.md` (committed on branch @ `<sha>`)
> - **Grooming history:** body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required
```

The plan exists on the branch. The branch is pushed. But the **architect's two-pass review never produced a GROOMED verdict** to write into the body. The post-flight recovery from mika#1123 saved the partial state (branch + plan callouts) but the canonical `Grooming history: /ce:plan → mika-arch first-pass (ITERATE/READY) → revisions → mika-arch second-pass (GROOMED)` literal is missing.

The `dispatch_no_grooming_marker` gate (`crates/mika-agent/src/skills/executor.rs:check_grooming_markers`) requires the literal substring `second-pass (GROOMED)` or `second-pass (READY, paraphrased GROOMED`. The post-flight recovery placeholder ("architect verdict not verified") does NOT match. Subsequent `ready` label dispatch fails the gate.

## Observed instances

| Date | Ticket | Pilot turns | Pilot $ | Outcome |
|------|--------|-------------|---------|---------|
| 2026-05-20 | mika#965 | 18 | $0.70 | Drift (different class — pilot didn't invoke /ce:plan; see sibling doc) |
| 2026-05-20 | mika#1224 | ? | ? | Wedge (post-flight recovered, no GROOMED verdict) |
| 2026-05-21 | mika#974 | ? | ? | Different class — pre-existing non-canonical recovery note from May 5 |
| 2026-05-21 | mika#1172 | ? | ? | Wedge (post-flight recovered, no GROOMED verdict) |

The 2 confirmed wedges (#1224, #1172) share identical body shape and identical mechanism. Pattern is reproducible enough to file as a systemic dev-groom skill defect.

## Root cause hypothesis (unverified)

The dev-groom skill's flow expects: `/ce:plan` → architect pass-1 → revisions → architect pass-2 → write `Grooming history` callout literal. Somewhere between pilot pass-1 completion and the body-callout write, the pipeline drops. Candidates:

1. **Pilot exits before pass-2 architect review**: the pilot's max_turns budget exhausts after pass-1, never runs pass-2. Post-flight recovery saves what was produced (branch + plan).
2. **Pass-2 architect call fails silently**: `mika ask --agent mika-arch` returns malformed JSON or empty response; pilot doesn't surface the error, exits Success.
3. **Body-callout writeback is conditional on pass-2 success**: if pass-2 doesn't produce a parseable verdict, the body write step is skipped entirely. Post-flight recovery substitutes its placeholder.

Per existing `feedback_body_callout_drift_two_classes.md` memory: drift class D ("fabricated SHA on uncommitted plan") was fixed by mika#1204. But this is a NEW class — not fabricated SHA, but missing verdict.

## Recovery (operator-side)

Re-fire the groom in a fresh turn:

```bash
mika ask --agent mika-dev "groom mika issue#N"
```

If the second attempt produces a clean callout: state was just an intermittent pass-2 failure. If it wedges identically: systemic dev-groom skill bug; file followup.

Alternative: manually run `/mika-ask-arch` against the existing plan to produce a verdict, then manually edit the body callout to match the canonical literal.

## Followup ticket candidates

1. **dev-groom pass-2 retry-or-escalate**: if architect pass-2 fails or returns malformed verdict, retry once with exponential backoff. On second failure, halt and escalate to operator (NOT silent post-flight recovery placeholder).
2. **Post-flight recovery awareness in dispatch gate**: when `check_grooming_markers` sees the literal "body callout recovered by post-flight" + "architect verdict not verified" pattern, return a specific error class (`dispatch_grooming_incomplete_post_flight`) so operator-visible messaging distinguishes "never groomed" from "groom started but didn't finish."
3. **Pilot turn budget for grooming**: if pilot is hitting max_turns mid-grooming, that's an observability gap. Track turn-budget exhaustion rate on dev-groom dispatches.
4. **Compound metric**: % of dev-groom dispatches that reach `(GROOMED)` verdict on first attempt. Below threshold → file alert.

## Counterexample: clean wedge-free grooms today

- mika#1077 (KG resolver obs) → groomed cleanly first try, shipped via PR #1236
- mika#963 (mika-test agent) → groomed cleanly, shipped via PR #1235
- mika#1218 → groomed cleanly, shipped via PR #1234

Pattern: tickets with **clean simple bodies** (no status notes, no recovery callouts, no historical context) groom cleanly. Tickets with **historical residue in the body** (status notes, prior recovery markers, milestone-deprioritization notes) tend to wedge.

This suggests the body content itself is a contributing factor — possibly pilot context-saturation when body is long/complex.

## Related

- mika#1123 — body callout drift fix (introduces post-flight recovery placeholder)
- mika#1207 — milestone-close-claim guard parity (architect's-jailer, sibling architect-verdict class)
- `feedback_body_callout_drift_two_classes.md` — memory enumerating four drift classes; this is potentially class E
- `2026-05-21-pilot-drift-on-deprioritization-status-notes.md` — sibling doc (different class: pilot doesn't invoke /ce:plan AT ALL)
- `2026-05-21-dispatch-limit-exceeded-same-turn-wedges-groom-supervisor.md` — sibling doc (different class: guard 4 rejects same-turn dispatch)
