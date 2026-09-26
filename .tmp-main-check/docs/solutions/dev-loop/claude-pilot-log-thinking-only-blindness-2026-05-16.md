---
title: claude-pilot log emits zero events on thinking-only turns
module: claude-pilot
date: 2026-05-16
problem_type: observability_gap
component: dev_loop
severity: high
tags:
  - claude-pilot
  - logging
  - observability
  - pilot-drift
  - autonomous-loop
related_components:
  - claude-pilot-py
  - mika-dev
  - self-dev
applies_when:
  - "claude-pilot session executes turns that produce only ThinkingBlock or ToolUseBlock content"
  - "SDK billing/turn counters disagree with on-disk log file size"
  - "Operator attempts post-mortem of a drifted autonomous session"
---

# claude-pilot log emits zero events on thinking-only turns

## Symptom

A claude-pilot autonomous session reports substantial activity at the SDK level (multiple turns, non-trivial cost, wall-clock seconds) but the on-disk log file is essentially empty — only the `[init]`, `[prompt]`, and (if any) `[error]` events. No per-turn record exists for the work the SDK actually did. Operator cannot diagnose what the session was doing during its drift.

## Mechanism

`claude-pilot-py/src/claude_pilot/agent.py` iterates over the SDK's streamed content blocks and emits log events only when the block is a `TextBlock`:

```python
async for message in client.receive_response():
    for block in message.content:
        if isinstance(block, TextBlock):
            log_text(block.text)
        # ThinkingBlock: dropped
        # ToolUseBlock: dropped
        # ToolResultBlock: dropped
```

When the underlying model produces turns that contain only thinking content (extended-thinking mode) or only tool-use content without intermediate text narration, no `log_text()` call fires. The SDK's turn/cost accounting advances; the log file does not.

This is not a buffering or flush issue — the events are never *generated*. The log file accurately represents what the emitter chose to emit, which on thinking-only or tool-only turns is "nothing."

## Canonical instance (2026-05-16)

mika#920 dispatch session `8c1f21da-…`. Telemetry from the SDK callback:

- **Turns:** 20+
- **Cost:** $1.03
- **Wall time:** 81s

On-disk log file at `/var/log/claude-pilot/<session>.log`:

- **Size:** 561 bytes
- **Content:** `[init]` line, `[prompt]` line, `[error]` line at exit. No per-turn entries.

Operator attempted post-mortem to determine why the session failed to land any PR. The log file offered nothing beyond "it started and it ended" — the 20 turns of thinking and tool use in between were invisible. Root cause of the drift had to be inferred from secondary signals (no PR opened, no branch pushed, no `tasks` row state change) rather than read directly.

## Why this matters

Pilot drift is the canonical autonomous-loop failure mode in this workspace. Documented instances in the last six weeks:

| Date | Ticket | Drift shape |
|------|--------|-------------|
| 2026-04 | mika#940 | dispatched, no PR |
| 2026-04 | mika#960 | dispatched, no PR |
| 2026-05-13 | mika#1126 | dispatched, no PR |
| 2026-05-16 | mika#1142 | grooming recursed on bug report |
| 2026-05-16 | mika#920 | 20 turns, no PR, opaque log |

In every prior case, root cause was inferred from absence-of-artifact rather than read from logs. The log emitter's thinking-blindness means **every drift incident is post-mortem-undiagnosable from logs alone**. Diagnosis depends on the operator's pattern recognition over secondary signals — which scales poorly and is the bottleneck on closing the autonomous loop's reliability gap.

## Fix (in-flight)

mika#1142 (filed 2026-05-16). Recommended **Option A** — emit a single line per turn when no `TextBlock` content was produced:

```python
text_emitted = False
for block in message.content:
    if isinstance(block, TextBlock):
        log_text(block.text)
        text_emitted = True

if not text_emitted:
    log_text(f"[turn {n}] thinking-only, no actions")
```

Cheap to add (~5 lines), no schema change, no downstream-consumer change. Trades a small amount of log volume for the ability to see that turns are happening. A drifting session goes from "561-byte mystery" to "20 lines of `[turn N] thinking-only`," which makes the failure mode visible at a glance.

Option B (full ThinkingBlock/ToolUseBlock serialization) deferred — higher volume, parsing concerns, no current consumer demanding it. Option A is the cheapest move that retires the diagnostic gap.

## Related

- Ticket: mika#1142 (fix in flight).
- Sibling: `docs/solutions/dev-loop/cli-log-noise-stderr-pollutes-piped-commands-2026-04-17.md` (different log surface, same family: dev-loop observability).
