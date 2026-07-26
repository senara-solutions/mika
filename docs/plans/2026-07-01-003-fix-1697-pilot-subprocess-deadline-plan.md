---
type: fix
issue: 1697
title: claude-pilot subprocess deadline observability — pilot_deadline_warn + optional hard-kill
status: draft
---

# Plan — mika#1697 claude-pilot subprocess deadline observability

## Ticket

mika#1697 — a subsystem-boundary observability gap between the agent loop's 5-min deadline (in-process) and the claude-pilot subprocess (out-of-process, no deadline propagation). Today's mika#1655 and mika#1665 pilots ran 3h+ each without any warn/reap signal. Both eventually completed successfully (Hypothesis 1 of mika#1687 ratified). This ticket adds the structural surface: soft-warn at N× deadline, hard-kill at absolute threshold, task-health signal for the operator dashboard.

## Problem

`run_claude_pilot` spawns claude-pilot via the `claude-pilot-py` SDK as a long-running subprocess. The agent loop's 5-minute deadline (in `crates/mika-agent/src/loop.rs` or wherever `MAX_LOOP_ELAPSED` lives) governs the in-process Rust loop only — it does not propagate to the subprocess. The callback watchdog (`MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS`) only fires when the subprocess DIES. Result: a subprocess running fine for 3h+ is invisible to every existing timeout/health signal.

Concrete evidence (from ticket body): mika#1655's pilot ran continuously from 13:36:26Z → 16:54Z = 3h 18min. mika#1665 ran 14:48:31Z → ~18:30Z = 3h 42min. Both produced `wip(mika#16XX): impl staged by post-flight recovery` PRs on completion, proving continuous work throughout. No `pilot_slow` or `pilot_timeout_imminent` signal existed.

Resource cost per slow pilot: ~$1.75 in LLM billing (vs typical $0.10 fast pilot), 3h+ dispatch-slot lock (blocks concurrent implement-class work per agent per mika#525's slot-limit), zero operator observability.

## Scope

**In scope (v1 ships):**

1. **Soft-warn at N× agent deadline.** Configurable `MIKA_PILOT_SOFT_WARN_MULTIPLIER` (default 2 = 10min). On every callback-watchdog tick, for each `in_progress` task with `dispatch_class = 'implement'` whose subprocess has been alive > N × `MAX_LOOP_ELAPSED`, emit a structured WARN log event + `audit_events` row (kind = `pilot_slow`, target_key = task_id, detail = subprocess PID + elapsed seconds). Idempotent: emit once per task (or once per elapsed-doubling: 2×, 5×, 10× thresholds).

2. **Hard-kill at absolute threshold.** Configurable `MIKA_PILOT_HARD_TIMEOUT_SECS` (default 3600 = 60min). On watchdog tick, for each `in_progress` implement task whose subprocess elapsed > this threshold: force-kill the subprocess (SIGTERM, then SIGKILL after grace), mark the task `failed` with `error_reason = "subprocess_exceeded_hard_timeout"`, emit `audit_events` (kind = `pilot_hard_killed`).

3. **Task health signal.** Extend the task health summary (per `crates/mika-agent/CLAUDE.md` § Task health awareness) with `pilot_slow` (subprocess > 2× deadline) and `pilot_timeout_imminent` (subprocess > 80% of hard threshold). Surface in operator dashboard's task-list view.

4. **Dispatch slot release on hard-kill.** When hard-kill fires, the implement slot for that agent must be released so the next queued task can dispatch. Verify current slot-release logic on task-failure path releases correctly.

5. **Structured audit trail.** Every warn/kill emission writes an `audit_events` row so operator-CC can query history: how often does pilot_slow fire, which agents, what work classes.

**Out of scope:**

- Making pilots FASTER. This ticket adds observability + a safety-net; it does not investigate why a specific pilot is slow.
- Model-side latency analysis (folded into mika#1699's latency capture; see peer-review context).
- Alternative timeout regimes (per-work-class thresholds, adaptive based on prior runs).
- glm-5.2 specific latency workarounds.

## Acceptance criteria

Transposed from the issue body's "What's actually missing" section + F1 architect requirement (mika#1559 gate — this section must be present + non-empty):

- [ ] **AC1** — Soft-warn signal fires when subprocess elapsed > `MIKA_PILOT_SOFT_WARN_MULTIPLIER × MAX_LOOP_ELAPSED_SECS` (default: 2 × 300s = 10min). Emission is idempotent per crossed threshold (2×, 5×, 10×) — no duplicate warns per threshold per task.
- [ ] **AC2** — Hard-kill signal fires when subprocess elapsed > `MIKA_PILOT_HARD_TIMEOUT_SECS` (default 3600s = 60min). SIGTERM sent, 30s grace, then SIGKILL. Task marked `failed` with `error_reason = "subprocess_exceeded_hard_timeout"` + `retryable = false`.
- [ ] **AC3** — `TaskHealth` struct extended with `pilot_slow: bool` (subprocess > 2× deadline) and `pilot_timeout_imminent: bool` (subprocess > 80% of hard threshold). New fields use `#[serde(default)]` for backwards-compat with existing consumers (F4).
- [ ] **AC4** — Dispatch slot released immediately on hard-kill. Integration test verifies: queue a task, hard-kill it via elapsed-time SQL injection, assert next queued task can dispatch within one watchdog-tick. See F2 verification prerequisite.
- [ ] **AC5** — `audit_events` rows emitted per emission: `kind = "pilot_slow"` on soft-warn (target_key = task_id, detail = elapsed + PID + threshold), `kind = "pilot_hard_killed"` on hard-kill (target_key = task_id, detail = elapsed + PID + signal-sequence).
- [ ] **AC6** — Subprocess PID + start_time queryable via `tasks.metadata` JSON path (F3): `metadata.subprocess.pid` (i32) and `metadata.subprocess.started_at` (unix seconds). Uses existing `set_task_metadata_field` / `get_task_metadata_field` helpers per callback-watchdog (mika#959) precedent.
- [ ] **AC7** — Configuration documented: `.env.example` + `docs/configuration.md` describe both new env vars side-by-side with `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` to clarify semantic differences (life-of-subprocess vs post-death detection).
- [ ] **AC8** — Regression suite passes: `cargo test -p mika-agent` clean, `make calibrate-mika-dev` shows no calibration regression, existing fast-pilot dispatches complete normally with no false-positive soft-warn.

## Deliverables (mapped to ACs)

The ticket body doesn't list explicit AC1-N (this is a structural investigation-and-fix filing). Above `## Acceptance criteria` section transposes into testable form per F1. Deliverables mapped:

| Deliverable | File(s) |
|---|---|
| DP1 — Soft-warn at N× deadline, idempotent per threshold | `crates/mika-agent/src/tools/claude_pilot.rs` (or callback watchdog loop location) — add PID+start-time tracking per task; `crates/mika-agent/src/engine/mod.rs` watchdog tick — add elapsed-threshold check + emit path |
| DP2 — Hard-kill at absolute threshold | Same location as DP1 — add SIGTERM/SIGKILL logic + task failure marker |
| DP3 — Config vars | `crates/mika-common/src/settings.rs` — `MIKA_PILOT_SOFT_WARN_MULTIPLIER` (u32, default 2), `MIKA_PILOT_HARD_TIMEOUT_SECS` (u64, default 3600), documented in `.env.example` + `docs/configuration.md` |
| DP4 — audit_events kinds | `crates/mika-agent/src/audit/kinds.rs` (or wherever kinds live) — add `pilot_slow`, `pilot_hard_killed` variants |
| DP5 — Task health signal | `crates/mika-agent/src/tasks/health.rs` or equivalent — extend `TaskHealth` with `pilot_slow: bool`, `pilot_timeout_imminent: bool`; wire into dashboard task-summary endpoint |
| DP6 — Documentation | `crates/mika-agent/CLAUDE.md` § Callback watchdog subsection — document the new signals + env vars |
| DP7 — Tests | `crates/mika-agent/tests/` — unit test soft-warn threshold logic (mocked task with old subprocess start time); integration test hard-kill via `MockLlmProvider` or process-injection test harness (may require new mock) |

## Implementation steps (dispatch order)

**Phase 1 — Subprocess PID + start-time tracking via `tasks.metadata` JSON (F3 ratified).**
The callback watchdog (mika#959) already persists `process_start_time` in `tasks.metadata` via `set_task_metadata_field()`. Extend the same JSON path with `subprocess.pid` + `subprocess.started_at`. No DB schema change needed. Use existing `set_task_metadata_field(task_id, "subprocess", json!({...}))` and `get_task_metadata_field(task_id, "subprocess")` helpers.

**Phase 2 — Watchdog tick extension.**
Add the elapsed-threshold check to the existing callback watchdog loop. For each `in_progress` implement task with a live subprocess PID + start_time: compute elapsed = now - subprocess_started_at. If crosses soft-warn threshold and not already logged at that threshold: emit `audit_events` + WARN log. Idempotency via a `pilot_deadline_warn_thresholds_logged` bitmask (or column) on the task.

**Phase 3 — Hard-kill path + slot release verification (F2 BLOCKING pre-implementation).**

**Slot release pre-check.** Before authoring the hard-kill logic, verify current slot-release behavior on task failure:

```bash
grep -rn "release.*slot\|SlotHealth\|slot_guard" crates/mika-agent/src/
```

Follow the chain from `update_task_failed()` to confirm `SlotHealth::release()` (or equivalent) is called automatically on any task→failed transition. If YES: hard-kill path can rely on it. If NO: this ticket must add explicit slot-release in the hard-kill path (add to Phase 3 scope). Failure to verify creates a silent-wedge class (freed process, locked slot, next queued task blocked forever).

**Hard-kill body.** If elapsed > `MIKA_PILOT_HARD_TIMEOUT_SECS`: send SIGTERM to subprocess PID, wait 30s grace, then SIGKILL. Mark task failed with `error_reason = "subprocess_exceeded_hard_timeout"` + `retryable = false` (retry on the same slow work class would just re-trigger). Emit `audit_events` (kind = `pilot_hard_killed`, detail includes PID + signal-sequence). If slot-release requires explicit call per pre-check: invoke it here.

**Phase 4 — Task health signal.**
Extend `TaskHealth` struct with the two new bool fields. Update dashboard task-summary endpoint to include them. Optional: add UI badge in dashboard task list view.

**Phase 5 — Config + docs.**
Register `MIKA_PILOT_SOFT_WARN_MULTIPLIER` + `MIKA_PILOT_HARD_TIMEOUT_SECS` in `Settings`. Update `.env.example`. Update `docs/configuration.md`. Update `crates/mika-agent/CLAUDE.md` callback-watchdog section.

**Phase 6 — Tests.**
Unit tests for soft-warn threshold + idempotency. Integration test that mocks a long-running subprocess (may need a helper: `MockPilotSubprocess` that sleeps N seconds without producing callback, allowing test to synthetically age the start_time).

## Verification

- Manual test: set `MIKA_PILOT_SOFT_WARN_MULTIPLIER=1` and `MIKA_PILOT_HARD_TIMEOUT_SECS=600` (10min hard). Dispatch a fast task via `/mika` — should complete before either fires. Then dispatch a task designed to hang (or artificially age its start_time via SQL). Verify soft-warn fires at ~5min, hard-kill fires at 10min, task marked failed, slot released.
- SQL check post-manual-test: `SELECT * FROM audit_events WHERE kind IN ('pilot_slow', 'pilot_hard_killed');` — confirm rows exist.
- Dashboard check: task health summary shows `pilot_slow=true` during the slow window.
- Regression: existing fast-pilot dispatches complete normally, no false-positive warn.
- `cargo test -p mika-agent` — no regressions in existing tests.
- `make calibrate-mika-dev` — no calibration regression from the watchdog changes.

## Risks

1. **Subprocess PID retention.** If claude-pilot forks/execs internally, the PID we track may be the wrapper, not the actual work-doing subprocess. Kill sending to the wrapper may not propagate. Verify via `pstree` or `ps -f --forest` during dev-mode test.
2. **Grace period tuning.** SIGTERM → 30s grace → SIGKILL. If claude-pilot has cleanup work to do (flush audit_events, close DB connection, push in-flight commits), 30s may not be enough. Configurable via a third env var, or fixed at 30s and documented as "brutal enough for a subprocess ignored by 60min deadline."
3. **Task-failure semantics.** Marking a task `failed` with `error_reason = "subprocess_exceeded_hard_timeout"` — will the existing loop retry logic re-dispatch it? For hard-timeout kills, retry MAY be counter-productive (same slow work class will slow-time out again). Consider adding `retryable = false` on this specific failure kind.
4. **Dashboard schema change.** Adding fields to `TaskHealth` breaks any consumer expecting a fixed shape. Verify current `Serialize` derive is #[serde(default)] friendly, or version the schema explicitly.
5. **Backwards compat for MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS.** The existing 120s grace is for post-DEATH detection. The new `MIKA_PILOT_HARD_TIMEOUT_SECS` operates during LIFE. Distinct concerns, distinct env vars, but naming may confuse operators. Document both in docs/configuration.md side-by-side.

## Out of scope (repeated)

- glm-5.2 latency (mika#1699 folds latency capture into calibration).
- Investigation of why specific pilots are slow.
- Adaptive per-work-class thresholds.

## References

- mika#1687 — silent pilot death observation, Hypothesis 1 confirmed (this ticket is the structural follow-up)
- mika#1696 — wedge-day epic
- mika#1699 — permission-policy disambiguator, folds latency capture free (base-latency comparison)
- `crates/mika-agent/CLAUDE.md` § Agent loop — 5-min deadline reference
- `crates/mika-agent/CLAUDE.md` § Callback watchdog — existing watchdog pattern to extend
- `crates/mika-agent/CLAUDE.md` § Task health awareness — `dispatch_stale` precedent
- `crates/mika-agent/src/tools/claude_pilot.rs` — subprocess spawn location (verify)
- Peer review 2026-07-01 — noted "fold latency into mika#1699 for free comparison data, mika#1697 stays structural"
- Vincent's authorization 2026-07-01 to run deploy + keep grooming through night
