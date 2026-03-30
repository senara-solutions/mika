# Post-Callback Turn Optimization & Inter-Agent Communication

**Date:** 2026-03-30
**Status:** Ready for planning
**Triggered by:** Dev run audit of mika#323 (task `6de0dc3f`)

## Problem

The dev run audit of mika#323 exposed two systemic issues:

1. **Post-callback scope creep:** mika-dev burned 4 of 10 tool steps on unrelated work items (#344, #345, #276) after receiving the claude-pilot callback for #323. Hit `agent exceeded max tool steps` before properly handling the QA hold verdict.

2. **Incomplete verdict:** mika-qa emitted `hold` because CI checks were still pending — but had no mechanism to follow up when CI completed. The hold was stale within 2 minutes (CI passed), but no re-review was triggered.

Root cause: the self-dev prompt conflates two concerns in the callback turn — *resumption* (handle the active task's verdict) and *progress checking* (sweep other work items). These belong to separate turns by design.

## What We're Building

### 1. Scoped post-callback turns

Post-callback turns handle ONLY the task that triggered the callback. No sprint sweep, no checking other work items. Heartbeat already owns "check other work" — that's its defined job.

**Why not "active first, then sweep"?** "Reserve N steps" is prompt-level enforcement — LLMs will rationalize crossing it. Fragile by design.

**Why not "increase limit"?** Treats the symptom. Wandering behavior scales with budget; you'd hit the ceiling at 20.

### 2. Structured verdict taxonomy

Replace freeform `VERDICT: hold` with structured verdicts that make mika-dev's post-callback behavior deterministic:

| Verdict | Meaning | mika-dev action |
|---------|---------|-----------------|
| `pass` | Diff clean, CI green | Merge (or notify Vincent) |
| `hold[ci_pending]` | Diff clean, CI not yet done | Set reminder, re-check CI on fire |
| `hold[review]` | Issues found, needs fixes | Notify Vincent, pause |
| `block` | Hard failure | Notify Vincent, pause sprint |

### 3. Two-phase async verdict via reminder

Decouple diff review from CI completion using the existing reminder system:

| Phase | Actor | Action |
|-------|-------|--------|
| Phase 1 (within 120s) | mika-qa | Diff review only. Check CI once. If pending: emit `hold[ci_pending]`. |
| Phase 2 (on reminder) | mika-dev | Check CI. If green: re-delegate for final verdict (or self-merge if diff already passed). If pending: extend reminder. |

**Key properties:**
- `delegate_task` timeout stays at 120s — no infrastructure change
- mika-qa's diff review is never blocked on CI (orthogonal concerns)
- CI polling owned by mika-dev via reminder, not buried in a delegated task
- Uses existing reminder primitive — zero new infrastructure
- mika-dev is the merge owner (SRP) — correct for it to gate on CI

**Why not poll CI in the QA turn?** Encoding CI latency (1-4 min, variable) into `delegate_task` timeout is a leaky abstraction — transport-layer config shouldn't compensate for workflow concerns. If CI slows down (new check, flaky runner), you'd keep adjusting infrastructure.

**Why not fail-open on CI timeout?** Unsound by design — defeats the purpose of CI as a quality gate.

## Key Decisions

1. **Post-callback = active task only.** Sprint sweep moves to heartbeat. (SRP)
2. **Structured verdicts.** `pass`, `hold[ci_pending]`, `hold[review]`, `block`. Deterministic mika-dev behavior per verdict type.
3. **Reminder-based CI follow-up.** 5-6 min initial delay. mika-dev checks CI, re-delegates or merges.
4. **No `delegate_task` timeout change.** 120s is correct for the diff review phase.
5. **No step limit increase yet.** Validate scoped callback fits in 10. Instrument first, adjust after.
6. **Task system as inter-agent bus: deferred.** Design it when verdict patterns are stable and real multi-task coordination cases emerge. YAGNI until callbacks prove insufficient.

## Prerequisite: Reminder System Bug Fix

The `hold[ci_pending]` → reminder → CI re-check flow depends on reminders firing at the correct time. There is a known bug: **reminders deliver one day earlier than expected**.

**Likely root cause:** The reminder system operates entirely in UTC — `create_reminder` accepts UTC timestamps, cron expressions fire in UTC, `next_fire_from_cron` (using the `cron` crate) computes in UTC. The "one day earlier" symptom points to a timezone conversion issue:

- The LLM converts the user's local-time intent ("remind me Thursday") to UTC, but gets the date boundary wrong (e.g., "Thursday 00:00 UTC" = Wednesday evening in a UTC- timezone)
- Or the system prompt doesn't provide the user's timezone to the agent, so the LLM assumes UTC when the user means local time
- Or an off-by-one in the fire-at comparison in the task engine tick loop

**This must be fixed and verified before relying on reminders for the CI follow-up flow.** The mika-dev reminder for `hold[ci_pending]` uses short delays (5-6 minutes from now), which are less affected by day-boundary issues. But the underlying bug needs fixing regardless — it breaks user-facing reminders too.

**Investigation path:**
1. Check if the agent's system prompt includes the user's timezone
2. Reproduce with a specific reminder and check the stored `next_fire_at` vs expected
3. Check the engine's `dispatch_due_tasks()` comparison logic (is it `<=` when it should be `<`?)
4. File as a separate issue — this is a bug fix, not part of the callback optimization

## Scope

### Changes needed

| Component | Change | Repo |
|-----------|--------|------|
| self-dev system prompt | Remove sprint sweep from post-callback turn. Add verdict-type dispatch table. | mika-skills |
| qa-review skill prompt | Emit structured verdicts. `hold[ci_pending]` when diff clean but CI pending. | mika-skills |
| self-dev system prompt | Add reminder-based CI follow-up for `hold[ci_pending]` verdict. | mika-skills |

### Prerequisites (separate issues)

| Component | Change | Repo |
|-----------|--------|------|
| Reminder system | Fix "one day early" delivery bug — timezone handling in create_reminder + task engine | mika |

### Not in scope

- Step limit changes (validate first)
- Task-based inter-agent messaging (deferred)
- `delegate_task` timeout changes

## Open Questions

None — all design decisions resolved during brainstorm.
