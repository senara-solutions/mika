---
title: "Per-class slot split: lightest-touch concurrency for asymmetric dispatch classes"
date: 2026-05-11
category: best-practices
module: task-engine
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - A single-session-at-a-time guard becomes a cadence bottleneck for a specific dispatch class
  - Dispatch classes have orthogonal state-coherence requirements (e.g., grooming touches plan files; implementation touches code files)
  - The number of concurrent dispatch classes is small and bounded (currently two)
tags:
  - dispatch-class
  - concurrency
  - per-class-slot
  - autonomous-loop
  - task-engine
  - grooming
  - state-coherence
---

# Per-class slot split: lightest-touch concurrency for asymmetric dispatch classes

## Context

The autonomous loop's single-session-at-a-time guard (`global_dispatch_active` in `executor.rs`, mika#583) prevents two `run_claude_pilot` callbacks from racing on the same agent's session memory, message channel, and DB writes. This guard is load-bearing for agent state coherence.

When mika#996 added auto-groom-on-dispatch (grooming ungroomed tickets before dispatching dev-pilot), both grooming and implementation ran serially through the same guard slot. This added ~15-25 minutes per ungroomed ticket to the autonomous-loop cadence. For a 7-child milestone, the cumulative overhead was 90-150 minutes of wall-clock time.

The key insight: grooming and implementation are **asymmetric dispatch classes** with orthogonal state-coherence requirements. A grooming subprocess reads issue bodies and writes plan files; an implementation subprocess reads plans and writes code. They don't race on the same state surfaces.

## Guidance

When a single-session-at-a-time guard becomes a cadence bottleneck and the dispatch classes have orthogonal state-coherence profiles, split the guard by class rather than reaching for a worker-pool architecture.

The implementation shape (mika#1001):

1. **Schema column**: Add `dispatch_class TEXT` to the tasks table (nullable for backward compat with pre-migration rows). CHECK constraint limits values to the known classes.

2. **SQL-layer COALESCE**: The guard query uses `COALESCE(dispatch_class, 'implement')` to treat pre-migration NULL rows as the default class. This is SQL-layer, not application-layer — ensures direct DB queries, debugging sessions, and future tooling all see consistent semantics.

3. **Derive-and-set pattern**: A `derive_dispatch_class(skill)` helper maps skill names to classes at the call site. Callback tasks carry the class from creation. The mapping is a simple match — `"dev-groom" -> "groom"`, everything else -> `"implement"`.

4. **Task-reuse flip**: When a task transitions from grooming to implementation (mika#996's task-reuse pattern), `update_task_dispatch_class()` atomically flips the class before the next dispatch.

```rust
// Guard query becomes per-class:
"SELECT parent_task_id, id FROM tasks
 WHERE trigger_type = 'callback'
   AND status IN ('pending', 'in_progress')
   AND parent_task_id IS NOT NULL
   AND parent_task_id != ?1
   AND agent_id = ?2
   AND COALESCE(dispatch_class, 'implement') = ?3
 LIMIT 1"
```

## Why This Matters

The per-class slot split preserves all pre-existing state-coherence invariants while eliminating the serial grooming bottleneck. The async DB serialization (single OS thread + mpsc channel) prevents within-dispatch write races. The `task_id` field on each message partitions the timeline for forensic analysis of interleaved concurrent dispatches.

The alternative shapes considered and rejected:
- **Dedicated groomer agent (Option B)**: Stronger isolation but ~200-400 lines of new agent provisioning + cross-agent callback verification. YAGNI for the immediate cadence concern.
- **Worker pool (Option C)**: Most general (~600-1000 lines) but premature — the binary slot split handles the current need. Option C remains available if a third concurrent dispatch class becomes necessary.

## When to Apply

- The guard is per-agent scoped and the concurrent dispatches are on the same agent
- The dispatch classes are bounded and small (two classes: `implement` and `groom`)
- The classes have orthogonal state-coherence profiles (no shared write surfaces)
- The cadence cost of serialization is operationally significant (>10 min per dispatch)

**Contrapositive**: When dispatch classes share state-coherence requirements (e.g., two implementation dispatches both writing to the same code surface), per-class splits are unsafe. A worker-pool with per-worker isolated state is the right shape because each worker has its own session, and concurrent writes to the same state surface would race.

## Examples

**Before** (serial, mika#996 only):
```
groom ticket N    [15-25 min] → dispatch ticket N [90-150 min] → groom ticket N+1 ...
```

**After** (pipelined, mika#1001):
```
groom ticket N    [15-25 min] → dispatch ticket N [90-150 min]
                                 groom ticket N+1 [15-25 min] → dispatch ticket N+1 ...
```

Steady-state: grooming completes during the prior dispatch's execution window, so the cadence matches dispatch speed rather than `dispatch_time + groom_time`.

## Related

- mika#1001 — implementation ticket (this compound's source)
- mika#996 — auto-groom-on-dispatch (serial grooming, prerequisite)
- mika#583 — original `global_dispatch_active` single-session guard
- mika#22 — agent-lock-split design space (Bounded-B territory)
- `docs/solutions/best-practices/auto-groom-on-dispatch-2026-05-06.md` — companion compound for mika#996
