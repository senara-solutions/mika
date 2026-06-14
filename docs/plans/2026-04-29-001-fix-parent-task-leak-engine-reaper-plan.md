---
title: "fix: engine-side reaper for parent self_dev tasks orphaned by failed callbacks"
type: fix
status: active
date: 2026-04-29
ticket: senara-solutions/mika#871
branch: fix/871/parent-self-dev-task-leaks-in-progress
origin: 2026-04-28 mika#868 dev-run audit
related: senara-solutions/mika#870 (callback-turn assistant-message guard — sibling), senara-solutions/mika#862/#863/#864 (engine guard family — adjacent registry)
---

# fix: engine-side reaper for parent self_dev tasks orphaned by failed callbacks

## Overview

mika#871 is the engine-side safety net for the dispatch-reliability failure surfaced by tonight's mika#868 audit. mika#870 (sibling) adds a callback-turn post-condition guard requiring `update_task_status` + `send_message` before EndTurn. That guard cannot fire if the callback turn itself crashes (LLM transport error, max-tool-steps cap, deadline exceeded) before reaching EndTurn. This plan adds a periodic reaper to `TaskEngine::tick()` that detects parent self_dev tasks left `in_progress` after their `delivered` callback subtask produced no PR, and transitions them to `failed` with an audit-event trail.

The reaper is the structural complement to #870's prompt-driven guard. Both close the same failure class (silent dispatch failure) at different layers; either alone leaves a hole.

## Problem Frame

### Observed failure

From the 2026-04-28 audit on mika#868:

- Parent task `bd203e00-0ea8-47cd-913b-fa9427573837` (label `Implement mika#868: ...`, `source=self_dev`, `trigger_type=manual`) was `status=in_progress` since `2026-04-28T19:49:49Z`.
- Callback subtask `a60ac346-d4ef-4a0a-a6a5-25ee151962e2` (`trigger_type=callback`, `action_type=resume_agent`) transitioned to `status=delivered` at `2026-04-28T20:02:43Z`.
- No PR was produced. Parent task metadata empty (no `pr_url`, no `branch`, no terminal-state record).
- 12+ hours later, parent still `in_progress`. Nothing reaped it.

### Root cause (located in task_engine/)

From the Phase 1 Explore agent's mapping:

- **Periodic-scan loop** — `crates/mika-agent/src/task_engine/engine.rs:188` (`TaskEngine::spawn_tick_loop`) → `tick()` at line 204. DB scan fires every 60 ticks (60s) at line 213. Current scan calls four functions (lines 214-223): `expire_timed_out_tasks`, `kill_orphan_processes`, `scan_db_for_new_tasks`, `dispatch_undelivered_callbacks`. **None reconcile parent task state based on child callback outcomes.**
- **Existing health-summary detection without action** — `get_task_health_summary` (per `crates/mika-agent/CLAUDE.md`) detects 6 anomaly types including `stuck_callback` (completed but not delivered >10min), `stale_pending` (#583), but the output is only injected as a `<task-health>` block in heartbeat/callback-turn system prompts. The agent *might* act on it; the engine never does. The mika#868 audit instance is a textbook "agent saw the anomaly and didn't act" — exactly the failure mode #870 is closing on the prompt side.
- **Callback metadata missing PR signal** — `try_extract_callback_metadata` at `crates/mika-agent/src/task_engine/dispatcher.rs:914-973` extracts `session_id`, `turns`, `cost_usd`, `duration_ms` from the claude-pilot tail but does NOT extract `pr_url` or `branch`. The reaper cannot check parent metadata for "did claude-pilot ship a PR?" because that field was never recorded.
- **Status transitions enforced at tool layer, not DB layer** — `Database::update_task_status` at `crates/mika-agent/src/db.rs:4023` is a raw UPDATE with no state-machine check. The tool-layer `TransitionValidator` enforces `pending → any`, `in_progress → blocked/completed/cancelled`, etc. The reaper bypasses the validator (engine-side mutation is intentional) but must still write the audit event.

### Why this is p2 not p1

p1 went to #870 because its blast radius is "every claude-pilot timeout fails the same way *and* the operator has no signal." #871 is the safety net for the smaller class where #870's guard itself can't fire (turn crash, max-steps, deadline). With #870 in place, #871's reaper fires rarely — but it must fire reliably when needed because the alternative is silent unbounded accumulation of stuck parents in the operator's dispatch queue.

## Requirements Trace

- **R1.** New reaper function `reap_orphaned_parent_tasks` registered in `TaskEngine::tick()` (`crates/mika-agent/src/task_engine/engine.rs:204`) as the 5th periodic-scan call after `dispatch_undelivered_callbacks` at line 222-223. Runs once per 60-tick cycle (every 60s), same cadence as the existing scans.
- **R2.** Reaper-detection query: parents with `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'` whose latest callback subtask is `status='delivered'` AND `updated_at < (now() - REAPER_GRACE_SECONDS)` AND parent metadata does NOT contain `pr_url` AND no other active callback child exists. Grace period: **600 seconds** (10 minutes). **Calibration:** 600s = ~3× the upper bound of observed callback duration (mika#868 audit instance: 187s LLM latency on the silent-failure turn; max-tool-steps cap × per-step latency under #870's re-enter loop ≈ 200s upper). Long enough that #870's re-enter recovery completes; short enough that operator dispatch queue clears within one tick-cycle (60s) after grace expires. **`updated_at` semantics (F2 verification):** `mark_task_delivered` at `crates/mika-agent/src/db.rs:4661-4669` writes `status='delivered'` + `updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')` with a `WHERE status IN ('completed','failed')` guard — both `completed` and `failed` are loop-end terminal states (set at agent-loop conclusion, not at dispatch time), so `updated_at` on a delivered callback is loop-end-anchored. Mild imprecision: any post-delivery mutation of the child row would advance `updated_at` and delay reaping by another grace period. That bias is safety-favorable (touched-post-delivery rows are not reaped) and acceptable per YAGNI.
- **R3.** On match: transition parent to `status='failed'` via `Database::update_task_status` (`db.rs:4023`), inserting an `audit_events` row with `tool_name='task_engine_reaper'`, `target_key=parent.id`, `before_value='in_progress'`, `after_value='failed'`, `reasoning='callback_delivered_without_pr_url'`, `session_id='system-{agent_id}'`, fresh `trace_id` per `mika_common::trace::generate_trace_id()`.
- **R4.** Extend `try_extract_callback_metadata` (`task_engine/dispatcher.rs:914-973`) to parse `pr_url` from claude-pilot output. **Pattern source (F4 verification):** `mika/skills/bundled/dev-pilot/handlers/run.sh:81,398` already emits `PR: ${PR_URL}` lines, with comment at line 391 `# Appends a "PR: <url>" line so the self-dev callback can extract pr_url`. The handler's existing intent is exactly the integration this reaper consumes. Regex: `^PR:\s+(https?://github\.com/[^\s]+)` (multiline, case-sensitive `PR:` prefix anchored at line start). Result appended to `claude_pilot` metadata object so the JSON shape becomes `{"claude_pilot": {..., "pr_url": "https://github.com/owner/repo/pull/N"}}`. Without R4, the reaper's "no `pr_url` in metadata" check would fire on every successful run after grace.
- **R4-atomicity (F1 resolution):** R1 (reaper) + R4 (extraction) **MUST ship in the same PR**. R1 alone misfires on every healthy run after the grace window because `pr_url` is never written. R4 alone is dead code with no consumer. Both changes live in `crates/mika-agent/src/task_engine/` (engine.rs + dispatcher.rs) — same crate, same review surface. The implementation MUST land them in a single commit (or sequential commits within one PR), not split across PRs. Files-to-Modify table below enforces this by listing both changes under the same ticket scope.
- **R5.** Integration test in `crates/mika-agent/tests/task_engine/` (or `tests/eval/` if no `task_engine/` test module exists). Three cases: (1) **happy path** — parent has `pr_url` in metadata after callback delivers, reaper does NOT transition; (2) **failure path** — callback delivered, parent has no `pr_url`, > grace period elapsed, reaper transitions to `failed` and emits audit event; (3) **grace period** — callback delivered < grace period ago, reaper does NOT transition (defer to next tick).
- **R6.** No new DB columns or schema migrations. The required state (parent status, child status, child delivered_at, parent metadata) is already in the `tasks.metadata` JSON column and the existing status/timestamp columns.

## Proposed Fix

### Primary: reaper function in TaskEngine

**Where:** `crates/mika-agent/src/task_engine/engine.rs` — append a new method to the `TaskEngine` impl block alongside the existing four periodic-scan methods.

```rust
// Pseudocode aligned with existing scan-method shape
const REAPER_GRACE_SECONDS: i64 = 600;

impl TaskEngine {
    async fn reap_orphaned_parent_tasks(&self) -> Result<()> {
        let candidates = self.db
            .find_orphaned_parent_tasks(REAPER_GRACE_SECONDS)
            .await?;
        for parent in candidates {
            let trace_id = mika_common::trace::generate_trace_id();
            match self.db.update_task_status(&parent.id, "failed").await {
                Ok(_) => {
                    self.db.add_audit_event(AuditEvent {
                        agent_id: parent.agent_id.clone(),
                        session_id: format!("system-{}", parent.agent_id),
                        tool_name: "task_engine_reaper".to_string(),
                        target_key: parent.id.clone(),
                        before_value: Some("in_progress".to_string()),
                        after_value: Some("failed".to_string()),
                        reasoning: Some("callback_delivered_without_pr_url".to_string()),
                        trace_id: Some(trace_id),
                    }).await?;
                    // F6 backfill log line: surface pre-existing leaks reaped from before deploy.
                    let age_hours = compute_age_hours(&parent.created_at);
                    if age_hours > 24 {
                        info!(
                            parent_id = %parent.id,
                            callback_task_id = %parent.callback_task_id,
                            age_hours,
                            "task_engine_reaper: reaping pre-existing orphan (possible backfill from before reaper deployment)"
                        );
                    } else {
                        info!(
                            parent_id = %parent.id,
                            callback_task_id = %parent.callback_task_id,
                            "task_engine_reaper: transitioned orphaned parent to failed"
                        );
                    }
                }
                Err(e) => {
                    // F5 audit-event-on-error: reaper failures land in the audit log so operators
                    // catch the silent-reaper-failure failure mode without separate observability.
                    let _ = self.db.add_audit_event(AuditEvent {
                        agent_id: parent.agent_id.clone(),
                        session_id: format!("system-{}", parent.agent_id),
                        tool_name: "task_engine_reaper".to_string(),
                        target_key: parent.id.clone(),
                        before_value: Some("in_progress".to_string()),
                        after_value: None,
                        reasoning: Some(format!("reaper_db_error: {}", e)),
                        trace_id: Some(trace_id),
                    }).await;
                    warn!(parent_id = %parent.id, error = %e, "task_engine_reaper: db error during transition");
                }
            }
        }
        Ok(())
    }
}
```

The new DB method `find_orphaned_parent_tasks` (`crates/mika-agent/src/db.rs`) issues:

```sql
SELECT parent.id, parent.agent_id, child.id AS callback_task_id
FROM tasks parent
JOIN tasks child ON parent.id = child.parent_task_id
WHERE parent.status = 'in_progress'
  AND parent.source = 'self_dev'
  AND parent.trigger_type = 'manual'
  AND child.trigger_type = 'callback'
  AND child.action_type = 'resume_agent'
  AND child.status = 'delivered'
  AND child.updated_at < datetime('now', ?1 || ' seconds')
  AND (parent.metadata IS NULL
       OR json_extract(parent.metadata, '$.claude_pilot.pr_url') IS NULL)
  AND NOT EXISTS (
    SELECT 1 FROM tasks sibling
    WHERE sibling.parent_task_id = parent.id
      AND sibling.id != child.id
      AND sibling.status IN ('pending', 'in_progress')
  )
ORDER BY parent.id
```

The `?1` parameter is `-REAPER_GRACE_SECONDS` (sqlite datetime modifier). The `NOT EXISTS` subquery handles the rare case where #870's guard relaunched claude-pilot via `create_task` — that creates a sibling callback child, and the reaper should NOT fire while a fresh attempt is in flight.

**Registration:** Add to `tick()` at `engine.rs:222-223` immediately after `dispatch_undelivered_callbacks`:

```rust
if let Err(e) = self.reap_orphaned_parent_tasks().await {
    warn!(error = %e, "task_engine_reaper failed");
}
```

The `warn!`-and-continue pattern matches the existing scan callsites — a reaper failure does not crash the engine.

### Secondary: extend callback metadata extraction

**Where:** `crates/mika-agent/src/task_engine/dispatcher.rs:914-973` — `try_extract_callback_metadata` and its helper `extract_callback_fields`.

Add a new field extraction for `pr_url`. The exact regex depends on the claude-pilot output format — pin at implementation against `mika-skills/dev-pilot/handlers/run.sh` (or wherever the PR URL lands in the tail). Likely shape:

```rust
const PR_URL_PATTERN: &str = r"(?im)^PR:\s+(https?://github\.com/[^\s]+)";
```

Result is appended to the `claude_pilot` metadata object alongside `session_id`/`turns`/`cost_usd`/`duration_ms` so the JSON shape becomes `{"claude_pilot": {..., "pr_url": "https://github.com/owner/repo/pull/N"}}`. The reaper's `json_extract(metadata, '$.claude_pilot.pr_url')` check (R3) keys off that.

If claude-pilot's output format does not include a stable `PR:` line, fold this requirement into a separate ticket on `mika-skills/dev-pilot` to emit the line. The reaper's R3 query would then check a different metadata field. This is a Phase-1-explorer follow-up rather than a plan-blocker.

### Tests

**File:** `crates/mika-agent/tests/task_engine_reaper.rs` (new integration test) or appended to existing `tests/task_engine/` module if one exists. Use the in-memory `Database::open(":memory:")` pattern from existing task-engine tests.

Three cases:

1. **Happy path — pr_url present.** Insert parent (`in_progress`, `self_dev`, `manual`) and child (`callback`, `delivered` 1h ago). Set parent metadata to `{"claude_pilot": {"pr_url": "https://github.com/x/y/pull/1"}}`. Run `reap_orphaned_parent_tasks`. Assert: parent stays `in_progress`. No audit event for `task_engine_reaper`.
2. **Failure path — orphan reaped.** Same setup, parent metadata empty (or `{"claude_pilot": {"session_id": "..."}}` with no `pr_url`). Run `reap_orphaned_parent_tasks`. Assert: parent transitions to `failed`. One audit event with `tool_name='task_engine_reaper'`, `before_value='in_progress'`, `after_value='failed'`, `reasoning='callback_delivered_without_pr_url'`.
3. **Grace period — within window.** Same as (2) but `delivered_at` 5 minutes ago (< 600s grace). Run reaper. Assert: parent stays `in_progress`. Reaper produced no audit event.

Optional 4th case (defer-to-implementation): **active sibling — relaunched.** Setup (2) plus a sibling callback child with `status='in_progress'`. Run reaper. Assert: parent stays `in_progress` (the `NOT EXISTS` clause excluded it).

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/task_engine/engine.rs` | Add `reap_orphaned_parent_tasks` method to `TaskEngine` impl; register in `tick()` after `dispatch_undelivered_callbacks` (~line 222-223); define `REAPER_GRACE_SECONDS = 600` const at file scope. |
| `crates/mika-agent/src/db.rs` | Add `find_orphaned_parent_tasks(grace_seconds: i64)` method on `Database` returning `Vec<OrphanedParentTask>` (new struct with `id, agent_id, callback_task_id`). |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | Extend `extract_callback_fields` to parse `pr_url` from claude-pilot tail; append to `claude_pilot` metadata object. |
| `crates/mika-agent/tests/task_engine_reaper.rs` | New file — three test cases above (or append to existing `tests/task_engine/` module if present). |
| `CHANGELOG.md` | Add entry under "Fixed" — "Task engine now reaps parent self_dev tasks left `in_progress` when their callback subtask delivers without producing a PR. Closes #871." |

No schema changes. No new dependencies. No new env vars (grace period is a const; revisit if operator wants tunable).

## Verification

### Unit / integration

```bash
cd /data/workspace/mika-platform/.claude/worktrees/fix-871-parent-self-dev-task-leaks-in-progress/mika
cargo test -p mika-agent task_engine_reaper
cargo test -p mika-agent  # full suite
cargo clippy -- -D warnings
cargo fmt --check
```

### Synthetic dev-run reproduction

After merge:

1. Start mika-spirit with the patched binary.
2. Create a synthetic parent self_dev task and a delivered callback subtask via direct DB inserts (or via a contrived dispatch that exits without a PR).
3. Wait > 10 minutes (grace period). On the next tick (within 60s after grace expires), reaper fires.
4. Verify in `~/.mika/data/mika.db`:
   ```sql
   SELECT id, status FROM tasks WHERE id = '<parent_id>';  -- expect 'failed'
   SELECT * FROM audit_events
     WHERE tool_name = 'task_engine_reaper' AND target_key = '<parent_id>';  -- expect 1 row
   ```
5. Run `/mika-audit dev-run <task_id>`. The audit's "Red flags" section MUST flag the parent transition cleanly (not as "still in_progress").

### Backfill of pre-existing leaked parents

The `bd203e00-…` parent from the 2026-04-28 audit (and any other pre-existing leaks) will be reaped automatically on the first post-deploy tick after the grace period elapses. No migration script needed; the same reaper that catches future leaks catches the historical one.

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Reaper fires on a parent that's mid-relaunch via #870's `create_task` recovery path. | The `NOT EXISTS` sibling-callback clause excludes parents with active callback children. The grace period (600s) is also longer than #870's typical re-enter window. |
| `pr_url` extraction regex doesn't match claude-pilot's actual output format. | The reaper would fire on every successful run, false-positive-reaping good parents. **Mitigation:** R4's regex is pinned at implementation time against the actual `dev-pilot/handlers/run.sh` output. Test (1) (happy path with `pr_url` present) catches the regression at PR-review time. If pinning slips, ship the reaper without R4 and have the reaper key off a different signal (TBD per Phase 1 explorer follow-up). |
| Grace period too short — slow callback turns get reaped before they finish. | 600s is generous: the existing `STALE_FAILED_CALLBACK_MINUTES` window is the precedent for "longer than any healthy callback should take." If runtime evidence shows healthy turns crossing 600s, raise; tunable via const-bump-and-redeploy. |
| Grace period too long — operator dispatch queue stays clogged. | At 10 minutes, a single failed dispatch costs at most one tick-cycle (60s) worth of queue blockage after grace. Acceptable. |
| Reaper transitions a parent that the operator was about to manually triage. | Audit event records the transition and reasoning. Operator can transition back via `update_task_status` tool (terminal `failed` is NOT a frozen state — `pending` re-dispatch is allowed). |
| Concurrent tick runs (engine restart mid-tick). | `tick()` is sequential per the existing engine architecture; the next tick after restart re-evaluates. No cross-tick state to corrupt. |

## Out of Scope

- **mika#870 (callback-turn assistant-message guard).** Sibling — separate plan landed on `fix/870/mika-dev-callback-turn-dies-silently` branch. #870 closes the loud-failure path; #871 closes the silent-crash path. Both ship.
- **Audit-finding #4 (claude-pilot-py SDK init bug).** Vincent's call. Independent code path.
- **Operator-facing tunable for `REAPER_GRACE_SECONDS`.** Const-bump-and-redeploy is sufficient until operational evidence calls for runtime tuning.
- **Reap parents whose children are still `pending` past a separate timeout.** The existing `expire_timed_out_tasks` scan handles callback-side timeouts; if a callback never gets dispatched, it never lands in `delivered` and the reaper's query won't match. Separate failure mode, separate ticket if it surfaces.
- **Reaper for non-`self_dev` parents.** The query is intentionally scoped to `source='self_dev'` because that's the dispatch flow with the documented PR-as-success-artifact contract. Other long-running parents (team-engine, scheduled tasks) have different success contracts and would need separate detection logic.

## Open Questions for mika-arch

1. **Grace period default.** `REAPER_GRACE_SECONDS = 600` (10 min) is my proposal. Argument for: longer than any healthy callback turn observed; gives #870's recovery loop time to complete. Argument against: operator dispatch queue stays "stuck" for 10 min on every failed dispatch. Alternative: 300s (5 min). Defer-to-architect.
2. **`pr_url` extraction layer.** R4 places extraction in `try_extract_callback_metadata` (`task_engine/dispatcher.rs`). Alternative: have `mika-skills/dev-pilot/handlers/run.sh` write `pr_url` directly to the parent task metadata via `update_task_status` before claude-pilot exits. The skill-side path is cleaner (no regex parsing on the mika side) but couples skill to engine schema. Probably defer to a follow-up — for now the engine-side extraction is the simplest path.
3. **Reaper failure visibility.** R1's `warn!`-and-continue pattern matches existing scans, but a reaper that silently fails (DB error, etc.) has worse consequences than a `expire_timed_out_tasks` failure. Should this path emit a separate metric or audit-event-on-error so monitoring catches the silent-reaper-failure failure mode? Defer to architect.
4. **Naming alignment with #870 and #862/#863/#864.** This isn't an EndTurn guard (those are LLM-loop post-conditions). It's an engine-tick reaper. Naming `task_engine_reaper` is distinct from the EndTurn-guard naming family by design.

---

## Architect first-pass concerns (resolved in this revision)

This revision applies the six findings from mika-arch's first-pass review (session `da320154-dbfe-40ce-9a2e-b0fd80b4ad67`).

### F1 — Atomic R1+R4 deployment (BLOCKING, resolved)

R1 (reaper) and R4 (`pr_url` extraction) MUST land in the same PR. R1 alone misfires on every healthy run after grace because `pr_url` is never written; R4 alone is dead code. Both changes live in `crates/mika-agent/src/task_engine/` (engine.rs + dispatcher.rs) — same crate, same review surface. Plan now states this explicitly under R4-atomicity. Files-to-Modify table enforces by listing both under the same ticket scope.

### F2 — `delivered_at` semantics pinned (BLOCKING, resolved)

`mark_task_delivered` at `crates/mika-agent/src/db.rs:4661-4669` is the sole writer of the `delivered` status. Its WHERE clause `status IN ('completed','failed')` guards that the row is in a loop-end terminal state before the transition. `updated_at` is touched by `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')` at the time of the delivery transition — loop-end-anchored, NOT dispatch-time-anchored. The reaper's grace clock starts at delivery, not at dispatch. R2 now uses `child.updated_at` (no separate `delivered_at` column exists) and documents the safety-favorable bias from post-delivery mutations.

### F3 — Grace-period calibration (sharpening, applied)

Plan's R2 now states: "600s = ~3× the upper bound of observed callback duration (mika#868 audit instance: 187s LLM latency on the silent-failure turn; max-tool-steps cap × per-step latency under #870's re-enter loop ≈ 200s upper)." Future reviewers have a calibration anchor, not just "feels generous."

### F4 — `^PR:` regex source pinned (sharpening, applied)

Plan's R4 now cites `mika/skills/bundled/dev-pilot/handlers/run.sh:81,398` (PR line emission) and `:391` (the comment `# Appends a "PR: <url>" line so the self-dev callback can extract pr_url`). The handler's existing intent IS the integration this reaper consumes — R4 is not introducing a new contract, it is consuming an already-designed one. Regex `^PR:\s+(https?://github\.com/[^\s]+)` (multiline, case-sensitive) confirmed against actual handler output.

### F5 — Audit-event-on-error (sharpening, applied)

Reaper's error arm now emits an `add_audit_event` with `tool_name='task_engine_reaper'`, `after_value=None`, `reasoning='reaper_db_error: {e}'`. Operators see silent-reaper-failure failure mode in the existing audit log without new observability infrastructure. Cost: one extra `add_audit_event` call on the error path.

### F6 — Pre-existing-leak backfill log line (sharpening, applied)

Reaper logs a distinct `info!` when `age_hours > 24` to surface backfill scope post-deploy. The `bd203e00-…` from 2026-04-28 will be reaped on first post-grace tick after deploy and land in the audit log with the `pre-existing orphan` log message. Operator gets post-hoc visibility; no migration script needed.

---

## Architect verdict

- **First-pass (mika-arch session `da320154-dbfe-40ce-9a2e-b0fd80b4ad67`):** ITERATE. Two blockers (F1 atomic deployment, F2 `delivered_at` semantics) + four sharpenings (F3 calibration, F4 regex source, F5 audit-on-error, F6 backfill log). All resolved in this revision.
- **Second-pass (same session, continuity preserved):** GROOMED. All six findings resolved with structural evidence. Two remaining uncertainties correctly deferred as YAGNI (`json_extract` performance, `tasks.type` scope). One residual non-plan note: PR description must include a sentence naming the `NOT EXISTS` sibling-guard's interaction with #870's optional `create_task` terminal action — the guard defers to retries launched by #870's correction loop; only parents whose entire child graph is terminal are reaped. (Captured here so the implementer carries it into the eventual PR body.)
