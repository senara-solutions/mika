---
module: teams
tags: [team-runs, orphan-reaper, liveness, not-exists, terminal-state, tool-error-vs-err, a2a_call, implementer-finds-contradiction, autonomous-loop]
problem_type: false-failure-state
category: best-practices
---

# `team_runs` need a terminal-state writer of last resort — and "all delegations failed" is not detectable at the delegation layer

## Problem (mika#1652)

`team_runs` rows stayed in `status='running'` indefinitely when the normal finalizer never executed — the `mika-spirit` process died mid-run, the `run_team` future was dropped at the tool timeout, or the run errored before `finalize_and_shutdown`. The held row keeps the team slot occupied, so subsequent `run_team` calls on the same orchestrator block. Founding incident: team `d68fdcaf` (2026-06-29), where `a2a_call` 503s left a row stuck `running` for 80+ minutes and the only recovery was a manual SQL `UPDATE`.

This is the team-run analogue of the task-table gap mika#871 (`reap_orphaned_parent_tasks`) and mika#1162 (`complete_parent_tasks_on_callback_success`) already solved: a recoverable failure became *unrecoverable* because nothing wrote terminal state.

## Solution — a dedicated `team_runs` reaper (Path 2, not generalized)

A periodic sweep, sibling to the task reaper, on the same `DB_SCAN_INTERVAL_TICKS` (60-tick) cadence:

- `Database::find_stuck_team_runs(threshold_secs, liveness_threshold_secs)` (`db.rs`) — selects `status='running'` rows older than the stuck threshold whose child sessions show **no** `llm_calls`/`tool_calls` activity within the liveness window. Uses the mika#959 `NOT EXISTS` liveness pattern, adapted for non-process entities: team child sessions are LLM-call/tool-call rows, not subprocesses, so liveness is **row recency**, not a `/proc/<pid>/stat` check. The `session_id LIKE 'team-' || r.id || '%'` prefix matches both the orchestrator session (`team-<id>`) and per-member sessions (`team-<id>-<agent>`); run ids are UUIDs so the prefix never bleeds across runs.
- `Database::transition_team_run_terminal(id, status, reason)` — guarded idempotent `UPDATE ... WHERE id = ? AND status = 'running'`, returns `true` only when a row changed. Loses cleanly to the normal finalizer or an operator cancel that won the race.
- `teams::engine::reap_orphaned_team_runs(&AsyncDatabase)` — free function (not a `TeamEngine` method; the reaper is a periodic sweep, not per-run state), wired into `task_engine::engine::tick()`.

**Thresholds:** `STUCK = 20 min`, `LIVENESS = 5 min`. 20 min captures "genuinely wedged" without false-positing on slow-but-legitimate multi-agent coordination (members may legitimately go quiet waiting on workspace file I/O); 10× the 300s tool timeout (50 min) would be too conservative for the founding shape.

**Status is `'failed'`, not `'cancelled'`.** The reaper is a system-level failure detector, not an operator proxy; `'cancelled'` is reserved for operator-initiated termination. The `team_runs.status` CHECK constraint already allows both — no migration. **SOLE WRITER** of the `reaper.team_runs` audit `tool_name`.

## The load-bearing lesson — a tool error is `Ok(ToolOutput::error)`, not `Err`

The ticket's C-rider (AC4) asked `run_team` to early-fail "when all delegation attempts in an iteration fail at the transport layer." It was groomed READY with a check placed "after `parse_task_assignments()`, before the `join_set`," filtering `delegation.error.is_terminal_transport()`. **All three premises were contradicted by the code:**

1. **`is_terminal_transport()` does not exist** anywhere in the tree.
2. **The location has no delegation results.** `parse_task_assignments()` (`teams/engine.rs`) yields task *assignments* (who does what); delegation *outcomes* only exist after the `join_set` collection loop, stringified into `TaskStatus::Failed(String)`.
3. **`a2a_call` 503 surfaces as `Ok(ToolOutput::error(...))` *inside* the agent loop** (`tools/a2a_call.rs`), not as `Err` from `run_team_agent`. So a 503-afflicted member completes as `Ok(response)` → `TaskStatus::Completed`, **not** `Failed`. An "all-delegations-failed" check at the delegation layer would never fire on the motivating incident.

The general rule: **in this engine, tools report failure by returning `Ok(ToolOutput::error(...))` so the agent loop can see it as a tool result and continue.** An `Err` is reserved for loop-level infrastructure failure. Any guard or short-circuit that keys off "the delegation failed" (status `Failed`) will silently miss every tool-level failure — which is most of them. Detecting transport failure means inspecting `tool_calls` rows *within* a completed delegation, not the delegation's `Result`.

Because honoring the F1 "terminal-transport-only, not all-errors" sharpening requires that new tool-result-inspection infrastructure — which the ticket's "do not widen scope" rule precluded — AC4 was **split** to mika#1671 for an architect re-groom with the corrected model, rather than shipped against a code shape that doesn't exist. The reaper alone resolves the founding incident (it frees the stuck slot); AC4 is a latency optimization on top. This is the implementer-finds-contradiction boundary: *complete* a resolution's plain intent (the reaper), *route* the overturn of a resolution's mechanism (AC4) back to the architect.

## Where to look

- `crates/mika-agent/src/db.rs` — `find_stuck_team_runs`, `transition_team_run_terminal` (next to the other `team_runs` methods)
- `crates/mika-agent/src/async_db.rs` — async wrappers (not agent-scoped; `team_runs` is shared)
- `crates/mika-agent/src/teams/engine.rs` — `reap_orphaned_team_runs` + thresholds + tests
- `crates/mika-agent/src/task_engine/engine.rs` — `tick()` wiring, next to `reap_orphaned_parent_tasks` (#871) and `complete_parent_tasks_on_callback_success` (#1162)
- `crates/mika-agent/src/tools/a2a_call.rs` — the `Ok(ToolOutput::error)` failure shape

## References

- mika#871 — task orphan reaper (sibling pattern)
- mika#1162 — parent auto-completer (sibling pattern)
- mika#959 — callback liveness watchdog (`NOT EXISTS` pattern reused here)
- mika#1671 — AC4 split (run_team early-fail, needs architect re-groom)
- mika#1653 — a2a_call vs delegate_task tool selection (same incident lineage)
- Founding incident: team `d68fdcaf-faec-45e5-81f8-25da5c4626a8` (2026-06-29)
