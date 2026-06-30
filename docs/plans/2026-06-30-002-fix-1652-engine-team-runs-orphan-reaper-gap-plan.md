---
issue: 1652
type: fix
date: 2026-06-30
---

# Plan — fix(engine): `team_runs` orphan-reaper gap (mika#1652)

## Problem

`team_runs` rows stay in `status='running'` indefinitely when underlying calls fail. No terminal-state writer exists for the team-run lifecycle equivalent to:

- `reap_orphaned_parent_tasks` (mika#871) — orphan-task reaper
- `complete_parent_tasks_on_callback_success` (mika#1162) — parent auto-completer

Founding incident `team-d68fdcaf-faec-45e5-81f8-25da5c4626a8` (2026-06-29): 4× `a2a_call` 503s → row stuck `running` for 80+ minutes → team slot held → subsequent `run_team` blocked → user-facing apology. Manual SQL `UPDATE` was the recovery bridge. This plan is the permanent answer.

## Architectural lineage

- mika#871 — task-table orphan reaper (`reap_orphaned_parent_tasks`)
- mika#1162 — parent-task auto-completer on callback-success
- mika#959 — `/proc/<pid>/stat` liveness pattern for callback subprocesses
- Mika Prime bearing read (session `00000000-0000-0000-0000-000000000000`, 2026-06-29 ~14:50Z): "The deepest defect here is not 'a2a_call failed.' It's 'a recoverable failure became unrecoverable because nothing wrote terminal state.'"

## Decision point — Path 1 vs Path 2 (architect-bearing)

The ticket explicitly defers this: *"Diagnose before sizing. is `reap_orphaned_parent_tasks()` parameterizable over both `tasks` and `team_runs` tables?"*

### Path 1 — generalize the existing reaper

Extend `reap_orphaned_parent_tasks()` to scan both tables. Single tick + audit + DB connection. Compact if the existing function's shape genuinely supports it.

**Risks** the architect must weigh:
- `tasks` and `team_runs` have different lifecycle semantics (a task is a single LLM-call wrapper; a team_run is an N-iteration loop). Their "stuck-detection thresholds" differ structurally.
- Code-coupling: a future change to task-reaper logic could silently regress team-run-reaper behavior.
- Test coverage: one set of tests must cover both lifecycles.

### Path 2 — dedicated `reap_orphaned_team_runs()`

New function in `engine.rs` parallel to the existing one, same 60-tick cadence. Independent thresholds + audit event types. The two reapers live as siblings, not a generalized parent.

**Risks** the architect must weigh:
- Code duplication: two similar-shaped functions querying different tables.
- Audit query convention: `tool_name LIKE '%team_runs_reaper%'` vs `'%orphan_task_reaper%'`. Should they share a prefix?

### Architect verdict (session `6b2e7667-f4b6-4173-a44d-0453e0e5ffac`, first-pass READY): **Path 2**

Tasks and team_runs have substantively different lifecycle semantics:
- **Tasks** = single-turn LLM calls with deterministic timeout boundaries (300s tool timeout + queuing)
- **Team runs** = multi-iteration state machines where members may legitimately block on each other (workspace file I/O, not just LLM calls)

A generalized function would need bifurcated logic for (a) threshold calculation, (b) liveness detection (tasks check `/proc/{pid}`, team runs check `llm_calls`/`tool_calls` recency), (c) terminal transition semantics. The "duplication" is domain-specific logic that should remain legible.

## Implementation outline (Path 2 default — adjust if architect picks Path 1)

### Phase A — stuck-detection query

New DB method `find_stuck_team_runs(threshold_secs: i64, liveness_threshold_secs: i64) -> Vec<TeamRun>`:

```rust
// Pseudo-SQL — final form goes through codebase conventions
SELECT tr.* FROM team_runs tr
WHERE tr.status = 'running'
  AND tr.started_at < datetime('now', '-' || threshold || ' seconds')
  AND NOT EXISTS (
    SELECT 1 FROM llm_calls lc
    WHERE lc.session_id LIKE 'team-' || tr.id || '%'
      AND lc.created_at > datetime('now', '-' || liveness_threshold || ' seconds')
  )
  AND NOT EXISTS (
    SELECT 1 FROM tool_calls tc
    WHERE tc.session_id LIKE 'team-' || tr.id || '%'
      AND tc.created_at > datetime('now', '-' || liveness_threshold || ' seconds')
  );
```

Mirrors the existing reaper's `NOT EXISTS` shape (per mika#959 liveness pattern). Architect: confirm thresholds.

**Thresholds (architect-ratified):**
- `STUCK_THRESHOLD_SECS = 20 min` — captures "genuinely wedged" without false-positing on slow-but-legitimate multi-agent coordination (members may legitimately go quiet while waiting for workspace file I/O). 10× tool timeout would be 50min — too conservative for the founding-incident shape.
- `LIVENESS_THRESHOLD_SECS = 5 min` — matches tick cadence with headroom.

### Phase B — reaper function

```rust
pub async fn reap_orphaned_team_runs(db: &DbHandle, audit_logger: &AuditLogger) -> Result<()> {
    let stuck = db.find_stuck_team_runs(STUCK_THRESHOLD_SECS, LIVENESS_THRESHOLD_SECS).await?;
    for run in stuck {
        let reason = format!(
            "Reaper: no liveness from child sessions for {}s",
            LIVENESS_THRESHOLD_SECS
        );
        // Status is 'failed' (not 'cancelled'): reaper is acting as a system-level
        // failure detector, not an operator proxy. 'cancelled' is reserved for
        // operator-initiated termination (e.g., the manual SQL recovery applied
        // to the founding incident).
        db.transition_team_run_terminal(run.id, "failed", reason.clone()).await?;
        audit_logger.log(AuditEvent {
            tool_name: "reaper.team_runs".to_string(),  // dot-namespaced; sibling: "reaper.tasks"
            agent_id: run.agent_id.clone(),
            session_id: format!("team-{}-reaper", run.id),
            metadata: json!({ "team_run_id": run.id, "reason": reason }),
            ..Default::default()
        }).await?;
    }
    Ok(())
}
```

### Phase C — wire into `tick()`

Add to the engine's tick loop next to the existing `reap_orphaned_parent_tasks()` call. Use the same `DB_SCAN_INTERVAL_TICKS` cadence (60 ticks ≈ once per minute at the engine's tick rate).

### Phase D — `transition_team_run_terminal()` DB method

New DB method in `crates/mika-agent/src/db/team_runs.rs`:

```rust
pub async fn transition_team_run_terminal(
    &self,
    team_run_id: &str,
    status: &str,  // "failed" (reaper) or "cancelled" (operator-initiated)
    failure_reason: String,
) -> Result<()> {
    sqlx::query(
        "UPDATE team_runs
         SET status = ?, ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             failure_reason = ?
         WHERE id = ? AND status = 'running'"
    )
    .bind(status)
    .bind(failure_reason)
    .bind(team_run_id)
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

The `WHERE status = 'running'` guard prevents double-transition (idempotency).

**Pre-implementation check:** verify the `team_runs.status` CHECK constraint allows both `'failed'` and `'cancelled'` — implementer first task. If the schema only allows one, file a migration ticket and use the allowed status.

## C-rider: `run_team` early-fail on all-delegation-failed iteration

Per ticket's C-rider — "even with terminal-state writing in place, `run_team` should early-fail when all delegation attempts in this iteration fail rather than blocking to the full 300s timeout."

Architect-confirmed location: `crates/mika-agent/src/teams/engine.rs` in `execute_iteration()` or `decompose()`. The delegation-failure check belongs **immediately after `parse_task_assignments()` returns, before spawning the `join_set`** (i.e., catch the all-error case before allocating the iteration's full timeout budget).

**Architect F1 sharpening (load-bearing):** the check must filter to **terminal transport errors only** (503, transport-error, deadline-exceeded), NOT all errors:

```rust
// After each delegation attempt within an iteration:
if delegations.iter().all(|d| d.error.as_ref().map(|e| e.is_terminal_transport()).unwrap_or(false)) {
    // All delegations failed at the transport layer — short-circuit to failed
    return Err(TeamRunError::AllDelegationsFailed(iteration_index));
}
// NOTE: a member that runs successfully but returns business-logic-error
// (e.g., `{"error": "could not parse"}`) is a *completed iteration with
// error result*, NOT a failed delegation. Those keep the iteration going.
```

**Why this matters:** the founding incident was all-503 (transport failure → short-circuit appropriate). A different failure shape — say all members complete and return "I couldn't help" — should NOT short-circuit; the team should iterate.

This is a one-line section per ticket discipline; **not a separate ticket**.

## Acceptance criteria

- **AC1** — `team_runs` rows in `status='running'` with `started_at < (now - STUCK_THRESHOLD_SECS)` AND no LLM-call/tool-call activity on child sessions within `LIVENESS_THRESHOLD_SECS` are transitioned to `status='failed'`. `ended_at` set; `failure_reason` populated as `"Reaper: no liveness from child sessions for {LIVENESS_THRESHOLD_SECS}s"`.
- **AC2** — An audit event is written per transition. Queryable: `SELECT * FROM audit_events WHERE tool_name = 'reaper.team_runs'`. Each event includes the `team_run_id` in metadata.
- **AC3** — Regression replay: a test scenario simulates today's founding incident shape (team_runs row stuck `running`, child session silent past `LIVENESS_THRESHOLD_SECS`). Verify reaper transitions it to `'failed'`. Verify subsequent `run_team` calls on the same orchestrator can proceed (slot freed).
- **AC4** (C-rider, architect-sharpened) — `run_team` early-fails on **all-terminal-transport-failed** delegations within an iteration (not all-errors — see C-rider section). Test scenarios:
  - All-503 → short-circuit to `failed` within seconds.
  - All-business-logic-error (e.g., member returns `{"error": "..."}`) → iteration continues, does NOT short-circuit.
- **AC5** — Existing happy-path team_runs unaffected. Regression test: a successful team run (mirrors `team-49640b61` from the founding incident) completes cleanly with `status='completed'` and `ended_at` set normally; reaper does NOT fire on it.

## Files involved (architect-confirmed)

- `crates/mika-agent/src/teams/engine.rs` — add `reap_orphaned_team_runs()`, wire into `tick()`. The team-runs reaper colocates with the `TeamEngine` impl (team-domain entities). If this file doesn't exist (team logic currently in `task_engine/`), create it — the team engine deserves its own module boundary separate from single-task execution.
- `crates/mika-agent/src/db/team_runs.rs` — add `find_stuck_team_runs()` + `transition_team_run_terminal()`. Follows the pattern of `crates/mika-agent/src/db/tasks.rs` — dedicated module per table.
- `crates/mika-agent/src/teams/engine.rs` (same file) — C-rider AC4 early-fail check in `execute_iteration()` or `decompose()`, immediately after `parse_task_assignments()` returns, before spawning the `join_set`.
- Tests: mirror the existing `reap_orphaned_parent_tasks()` test setup (`crates/mika-agent/src/task_engine/engine.rs` test module). Add the AC3 (reaper-replay), AC4 (early-fail), and AC5 (happy-path) scenarios.

## Out of scope

- Trigger (`a2a_call` 503s on local agents) — separately tracked at mika#1653 (the architect-routed design ticket for `a2a_call` vs `delegate_task`).
- D-class observation (team-run completing without delegation in run #1) — observation-only, no ticket.
- Refactoring the existing `reap_orphaned_parent_tasks()` if Path 2 wins. The tasks reaper stays as-is.

## Verification

- New DB migration NOT needed — `team_runs` schema already has `ended_at` and `failure_reason` columns. Implementer MUST verify the status CHECK constraint allows `'failed'` before committing the reaper.
- Architect first-pass: **READY** at session `6b2e7667-f4b6-4173-a44d-0453e0e5ffac` (Path 2, 20min/5min thresholds, `'failed'` status, `reaper.team_runs` audit name, F1 sharpening on C-rider).
- Architect F2 (documentation): this plan references mika#959's liveness pattern explicitly — the SQL `NOT EXISTS` form adapts it for non-process entities. The `/proc/<pid>/stat` form mika#959 ships is for callback subprocesses; team_runs aren't subprocesses (child sessions are LLM-call rows in `llm_calls`/`tool_calls`), so the adaptation is by-design.

## Implementation status (mika#1652)

Delivered per the architect-ratified decisions (Path 2, 20min/5min thresholds, `'failed'` status, `reaper.team_runs` audit name):

- **AC1, AC2, AC3, AC5 — shipped.** Reaper + DB layer + tick wiring + 6 tests.
- **AC4 (C-rider) — split to a follow-up (mika#1671).** Implementing the reaper surfaced that AC4 is unimplementable as groomed: (1) `is_terminal_transport()` does not exist in the codebase; (2) the specified location (after `parse_task_assignments()`, before the `join_set`) has no delegation results — those only exist post-`join_set`, stringified into `TaskStatus::Failed(String)`; (3) the founding-incident shape (`a2a_call` 503 → `ToolOutput::error` *inside* the agent loop, `a2a_call.rs:131`) produces `Ok`/`Completed` delegations, not failed ones, so an all-delegations-failed check would never fire on the motivating case. Honoring the F1 "terminal-transport-only" sharpening needs tool-call-result inspection (new infra), which #1652's "do not widen scope" rule precludes. The reaper alone resolves the founding incident (frees the stuck slot); AC4 is a latency optimization routed to the architect for re-groom with the corrected code model. See mika#1671 for full evidence.

**File-location corrections vs. plan (confirmed during implementation):**
- DB methods live in the monolithic `crates/mika-agent/src/db.rs` (+ async wrappers in `async_db.rs`), not a `db/team_runs.rs` module — the plan flagged this to "confirm."
- `team_runs.status` CHECK constraint (`db.rs:1129`) already allows `'failed'` — no migration needed (as the plan predicted).
- Reaper is a free function `reap_orphaned_team_runs(&AsyncDatabase)` in `teams/engine.rs` (honoring the architect's team-domain module-boundary choice), called from `task_engine/engine.rs::tick()` at the `DB_SCAN_INTERVAL_TICKS` (60-tick) cadence. It is not a `TeamEngine` method — the reaper is a periodic sweep, not per-run state.

## References

- mika#871 — task orphan reaper (sibling pattern)
- mika#1162 — parent auto-completer (sibling pattern)
- mika#959 — callback liveness watchdog (`NOT EXISTS` pattern reused here)
- mika#1653 — team-mode dispatch tool selection (architect-routed sibling; same incident lineage)
- Mika Prime bearing 2026-06-29 ~14:50Z (session `00000000-0000-0000-0000-000000000000`)
- Founding incident: team `d68fdcaf-faec-45e5-81f8-25da5c4626a8`, parent session `67c1245b-456a-4c87-9c3e-c2aa5de77d47`
