---
title: "dispatch_limit_exceeded silently wedges groom supervisor when same-turn implement + groom dispatch overlap"
date: 2026-05-21
category: runtime-errors
module: skills/executor
problem_type: runtime_error
component: dispatch
symptoms:
  - "groom task transitions to 'in_progress' but no claude-pilot child task ever appears under it"
  - "mika-dev's webhook turn says 'Dispatched grooming for mika#N' but pilot never runs"
  - "tasks.result on the groom supervisor shows {error: dispatch_limit_exceeded, dispatches_this_turn: 1}"
  - "Body callout never picks up a new (GROOMED) verdict — stays at whatever it had before the dispatch"
  - "Operator sees groom is 'awaiting architect' indefinitely"
root_cause: configuration_error
resolution_type: workflow
severity: high
tags:
  - dispatch
  - groom
  - guard-4
  - dispatch_limit_exceeded
  - same-turn
related:
  - mika#1001
  - mika#583
  - mika#1218
---

## Symptoms

A `mika ask --agent mika-dev "groom mika issue#N"` returns the canonical "Dispatched grooming for mika#N. Awaiting architect verdict via callback (task: `<id>`)" but `tasks.result` on that supervisor task contains:

```json
{"dispatches_this_turn":1,"error":"dispatch_limit_exceeded","reason":"Only one long-running dispatch is permitted per agent turn. A dispatch has already been launched in this turn. Wait for the current dispatch to complete via callback before launching another.","task_id":"<id>"}
```

The supervisor task sits at `in_progress` (or `blocked`) indefinitely, with **zero child tasks** under it. No claude-pilot subprocess ever spawns. The grooming body callout never gets a `second-pass (GROOMED)` verdict.

## Trigger pattern

Observed 2026-05-20: orchestrator-Claude fired both an implement-class dispatch (apply `ready` label on a groomed ticket) and a groom-class dispatch (`mika ask --agent mika-dev "groom <other-ref>"`) **in close succession**, both routed into the same mika-dev webhook turn. The turn-level dispatch counter (guard 4 in `validate_dispatch_readiness`) caps at 1 long-running dispatch per turn. The first dispatch (implement) ran; the second (groom) was rejected with `dispatch_limit_exceeded`.

The rejection writes the error to `tasks.result` (per #1108) but does NOT auto-retry on the next turn and does NOT escalate visibly. Mika-dev's own LLM correctly reports "Dispatched grooming for mika#N..." in its conversational reply because `run_claude_pilot_groom` returned a task ID — but downstream of the actual `executor.rs` guard, that task got rejected.

## Root cause

`crates/mika-agent/src/skills/executor.rs:validate_dispatch_readiness()` guard 4 (per-turn dispatch counter) is correctly rejecting the second long-running dispatch on the same turn, per the #583 contract. The gap is in **observability + recovery**:

1. The rejected dispatch's error JSON is captured in `tasks.result` but the LLM's surface response doesn't reflect the failure (it reports the task ID as if dispatch succeeded).
2. No automatic retry on a fresh turn. The supervisor task stays `in_progress` with no child claude-pilot task.
3. mika-dev's webhook handler doesn't periodically promote pending-but-rejected groom tasks back through dispatch.

## Recovery

**Operator-side:** re-fire `mika ask --agent mika-dev "groom mika issue#N"` in a fresh turn. The supervisor task (same task_id) gets re-dispatched and the `dispatches_this_turn` counter is fresh, so guard 4 passes.

Verify recovery:
```bash
sqlite3 ~/.mika/data/mika.db "SELECT id, status, substr(label,1,40) FROM tasks WHERE parent_task_id='<groom-supervisor-id>' ORDER BY created_at DESC;"
```

A new `long_running:run_claude_pilot_groom` child task should appear after the re-dispatch.

## Avoidance

When orchestrating, separate implement-class and groom-class dispatches by **at least one mika-dev conversation turn**. Pattern:

```
# Good — separate turns:
gh issue edit N --add-label ready                 # → webhook → turn 1 (implement dispatch)
sleep 30                                          # let turn 1 complete
mika ask --agent mika-dev "groom mika issue#M"    # → turn 2 (groom dispatch)

# Bad — same turn risk:
gh issue edit N --add-label ready
mika ask --agent mika-dev "groom mika issue#M"    # webhook + ask collapse into one turn → guard 4 fires
```

Or accept that the second will fail and proactively re-fire after observing the rejection in `tasks.result`.

## Related

- mika#583 — per-turn dispatch counter (introduced guard 4)
- mika#1001 — per-class dispatch slots (groom + implement can run concurrently across turns, but not in same turn)
- mika#1108 — dispatch-rejection observability (writes error to `tasks.result`)

## Follow-up ticket candidates

1. **mika-dev callback handler retry**: when supervisor task has `dispatches_this_turn: 1` in `result` and no child task exists, the next mika-dev turn should re-attempt dispatch (since the counter is per-turn, the next turn has a fresh counter).
2. **LLM surface fidelity**: mika-dev's response "Dispatched grooming for mika#N" is technically a lie when guard 4 rejected the spawn. Either suppress the success-shaped response when the underlying tool returned `dispatches_this_turn: 1`, or include the rejection reason in the user-facing reply.
