---
title: "claude-pilot grooming session drifts into executor mode when issue body opens with deprioritization status notes"
date: 2026-05-21
category: agent-quality
module: dev-groom
problem_type: behavior_drift
component: dev-pilot
symptoms:
  - "claude-pilot run completes with exit 0 (Success) but no plan file produced under docs/plans/"
  - "Pilot turns spent on `git log` / `git branch` investigation rather than /ce:plan invocation"
  - "Pilot session log shows zero /ce:plan or /ce:work skill activations"
  - "Body callout never updates with grooming-history entry"
  - "Supervisor task transitions to 'failed' or 'blocked' via 'pipeline_incomplete' callback heuristic"
root_cause: prompt_drift
resolution_type: workflow
severity: medium
tags:
  - dev-groom
  - claude-pilot
  - status-note
  - executor-drift
related:
  - mika#965
  - mika#1133
  - mika#1218
---

## Symptoms

Operator dispatches `mika ask --agent mika-dev "groom mika issue#N"`. Supervisor task transitions to `in_progress`. claude-pilot subprocess spawns and runs for 5-15 minutes (15-50 turns, $0.50-$1.50). Subprocess exits with status `Success` per the log's `[done]` line. BUT:

- Zero `docs/plans/YYYY-MM-DD-*-plan.md` file produced
- Zero `/ce:plan` skill invocation in the session log
- Branch may or may not be created; even if created, no commits
- HEAD unchanged on the worktree branch
- Body callout NOT updated

Mika-dev's callback handler then writes a "pipeline_incomplete" result to the supervisor task: `mika#N groom failed — Session drifted into executor mode. No plan file produced (no docs/plans/YYYY-MM-DD-*-plan.md >500 bytes), no /ce:plan invocation detected. HEAD unchanged.`

## Trigger

Observed 2026-05-20: `mika ask --agent mika-dev "groom mika issue#965"` — the issue body's FIRST PARAGRAPH was a status note:

```
## Status note (2026-05-06)

**Removed from milestone #22** (deprioritized 2026-05-06). The milestone's framing was
concurrency-for-velocity; that framing is retired (loop stability beats loop speed;
velocity is not a design driver for this project). This ticket retains its
**standalone callback-correctness justification** independent of any parallelism agenda.
```

The grooming pilot's first read of the issue body anchored on the deprioritization framing rather than the still-applicable AC. The pilot spent its turns investigating whether the ticket was still relevant (`git log` searches for related work, `git branch` reconnaissance) rather than producing a plan. Pilot completed "successfully" by its own definition (no errors, clean exit) but produced no grooming artifacts.

## Root cause

claude-pilot's grooming-mode prompt (the `/mika-groom-ticket` skill flow) anchors on the FIRST CONTENT in the issue body when forming its initial framing. When the first paragraph reframes the ticket as "deprioritized but retained for X reason," the pilot interprets the work as "investigate relevance" rather than "produce a plan for the still-applicable AC."

Existing engine guards do not catch this:
- `dispatch_no_grooming_marker` (#919) checks that the body has plan/branch/groomed callouts BEFORE dispatch — but the body had nothing groomed yet, so this guard is N/A
- post-flight callback heuristic detects "no plan file + no /ce:plan invocation" — fires correctly here, marks task blocked — but only AFTER $0.50+ has been burned

## Recovery

**Operator-side:** edit the issue body to move the status note BELOW the active AC. Repost via `gh issue edit N --body-file <fixed-body.md>`. Re-dispatch via `mika ask --agent mika-dev "groom mika issue#N"`.

Alternative: pick a different ticket. The deprioritized one may genuinely be lower-priority.

## Avoidance

When filing or editing tickets that retain a partial scope:

- Lead the body with the still-active acceptance criteria.
- Put status / deprioritization notes UNDER the AC, prefixed with `## History` or `## Status note (YYYY-MM-DD, contextual)`.
- Use blockquote (`>`) for status notes so they visually de-emphasize.

Example shape:

```markdown
## Description
<still-active AC>

## Acceptance criteria
- [ ] <ac1>
- [ ] <ac2>

---

> **Status note (2026-05-06):** Removed from milestone #22 (deprioritization framing).
> Standalone justification retained.
```

## Follow-up ticket candidates

1. **Grooming pilot prompt hardening**: add an explicit "find the active AC first, even if the body opens with status/deprioritization notes" instruction to `dev-groom`'s system prompt.
2. **Pre-dispatch body audit**: extend the `dispatch_no_grooming_marker` guard family with a "status-note-at-top" detection that warns (not rejects) before dispatch.
3. **Cost cap on grooming**: if a grooming pilot exceeds N turns without invoking `/ce:plan`, mark as drift and halt early (saves the $0.50-$1.50).

## Related

- mika#965 — original drift case (deprioritization status note at body top)
- mika#1218 — engine guard that fires on milestone-advance hallucinations (sibling concept: detecting drift via post-condition)
- `feedback_groom_ticket_no_halt_on_iterate.md` — memory on grooming dispositions (different class — ITERATE vs drift)
