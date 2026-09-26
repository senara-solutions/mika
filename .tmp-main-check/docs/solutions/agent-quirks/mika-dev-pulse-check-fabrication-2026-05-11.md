---
module: mika-agent
tags: [pulse-check, fabrication, task-health, anomaly-detection, wedge-detection]
problem_type: agent-fabrication
category: agent-quirks
issue: 980
---

# mika-dev pulse-check fabrication: "nothing stuck" during wedged iteration loop

## Problem

mika-dev's pulse-check answers ("anything stuck?", "what's the queue state?") can confidently report "nothing stuck" while the iteration loop is wedged — 6 consecutive `run_claude_pilot` failures (`success=0`) over 1h46m, with the agent claiming "in flight" 2 minutes before manual audit confirmed the wedge.

The root cause: the `get_task_health_summary()` function's 6 anomaly types all query the `tasks` table. When the iteration loop is wedged, the task row stays `in_progress` (not `blocked`, not `failed`), so no anomaly fires. The agent's verbal pulse relies on task status, which doesn't reflect tool-call failure patterns.

Same failure-mode family as mika-arch's persistence-meta hallucination (mika#947): **"describes deliverable instead of producing one."** mika-arch claimed findings existed when they didn't; mika-dev claims a queue is healthy when iteration is actively failing.

## Solution

Added two new anomaly types (#7) to `get_task_health_summary()` — dual-signal wedge detection that cross-checks the `tool_calls` table:

**Signal A (`dispatch_failures`):** Queries `tool_calls` for recent `run_claude_pilot` failures. When `COUNT(*) >= 3` within a 2h sliding window, the anomaly fires. Uses `session → task` JOIN for task correlation — attaches the anomaly to the specific task whose session produced the failures, not a heuristic lookup.

**Signal B (`dispatch_stale`):** Detects wedges where the agent stopped retrying entirely. When an `in_progress` manual task exists AND no `run_claude_pilot` call (success or failure) has been attempted in >1h, the anomaly fires. This is the aging defense — if Signal A's failures age out of the 2h window because the agent gave up, Signal B catches the silence.

**Mutual exclusion:** Signal A suppresses Signal B (prevents double-anomaly when both conditions are true).

## Why dual-signal matters

A single 2h sliding window creates a silent-drop risk: if failures age out AND the agent stops retrying, all evidence disappears. The dual-signal design ensures there is no time window where a wedged state produces a clean pulse:

- **Active failure stream:** Signal A fires (3+ failures in 2h)
- **Stale wedge:** Signal B fires (no dispatch attempt in >1h)
- **Transition zone:** As failures age out of Signal A's window, if no new attempts occur, Signal B activates within 1h

## Key design decisions

1. **tool_calls, not tasks** — The existing 6 anomalies query `tasks` with the `query_anomalies` helper (6-column row shape). The new query operates on `tool_calls` (different table, different shape) with standalone `TaskHealthAnomaly` construction.

2. **No prompt.rs changes** — The prompt injection at `prompt.rs:950-964` renders all anomalies generically via `anomaly_type`. Any new type appears automatically in the `<task-health>` block. The `<task-health-instructions>` block already instructs the agent to "review each anomaly" and "notify the user with anomaly details."

3. **Threshold: 3 failures in 2h** — Per the ticket's detection rule. Both values are constants in `health_thresholds`.

4. **Tool scope: `run_claude_pilot` only** — The ticket is specifically about dispatch wedges. Expandable to other long-running tools in the future.

## Files changed

- `crates/mika-agent/src/task_engine/types.rs` — `DISPATCH_FAILURE_THRESHOLD` (3), `DISPATCH_FAILURE_WINDOW_SECS` (7200)
- `crates/mika-agent/src/db.rs` — Anomaly #7 query block + doc comment + 10 unit tests
- `crates/mika-agent/CLAUDE.md` — Updated anomaly type list (6 → 8)

## Lessons

- **Cross-table correlation beats single-table status** — Task status alone is an unreliable health signal for long-running dispatch loops. The `tool_calls` table carries the ground truth of what actually happened.
- **Aging defense is mandatory for sliding-window anomalies** — A single time window always has a silent-drop edge. Dual-signal (or marker-based) designs eliminate the gap.
- **Same failure-mode family compounds** — This is the third instance of "agent claims X when reality is ¬X" (mika-arch persistence-meta, mika-arch gate evasion, mika-dev pulse fabrication). All fixed by making the engine inject ground-truth data into the prompt rather than relying on LLM reasoning about abstract state.
