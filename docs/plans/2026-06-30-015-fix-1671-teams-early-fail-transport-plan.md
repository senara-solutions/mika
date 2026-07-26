---
issue: 1671
type: fix
date: 2026-06-30
---

# Plan — fix(teams): run_team early-fail on all-terminal-transport-failed iteration (mika#1671 split from mika#1652)

## Problem

mika#1652's C-rider (AC4) — "`run_team` should early-fail when all delegation attempts in an iteration fail at the transport layer rather than blocking to the full timeout" — was unimplementable as the mika#1652 architect groomed it. Three concrete contradictions surfaced during implementation:

1. **`is_terminal_transport()` does not exist.** No such method in the codebase. Grep returns only `mika-a2a/src/state_machine.rs::is_terminal(TaskState)` (different concept), `agent_loop/mod.rs::is_terminal_tool_error` (tool-result classification, not delegation outcomes), and `is_terminal_disposition` (verdict-shape classification).

2. **Specified location has no delegation results.** mika#1652's plan placed the check "immediately after `parse_task_assignments()` returns, before spawning the join_set" — but `parse_task_assignments()` (engine.rs:1718) produces task *assignments*, not delegation *results*. Delegation outcomes only exist after the join_set collection loop (engine.rs:1242-1291) — and at that layer, each result is stringified into `TaskStatus::Failed(String)` (line 1252), so no structured error survives.

3. **The founding-incident shape never produces `TaskStatus::Failed`.** `a2a_call` 503s return `ToolOutput::error(...)` — a *tool result inside the agent loop* (tools/a2a_call.rs:146-153, the `send_message` `Err` arm). The agent's `run_loop` returns `Ok(response)`, so the delegation completes as `TaskStatus::Completed` (engine.rs:1247-1248, the `Ok(_)` arm), not `Failed`. An "all-delegations-failed" check at the join_set boundary would NEVER fire on the motivating case (Litha team `d68fdcaf-…`).

Inspecting `tool_calls` rows *inside* each completed delegation for transport-error signatures requires new infrastructure that mika#1652's plan's "one-line section" framing and its "do not widen scope" rule both preclude. Hence this split.

mika#1652's reaper (AC1-3, AC5) shipped and resolves the founding incident (stuck team_runs row freed). AC4 is a latency optimization that needs architect re-grooming with the corrected code model.

## Architectural lineage

- mika#1652 — parent ticket (reaper shipped at AC1-3+AC5, AC4 deferred here)
- mika#1653 — sibling (a2a_call vs delegate_task tool selection, CLOSED)
- mika-arch session `6b2e7667` (mika#1652's first-pass — the framing this fix corrects)
- Founding incident: team `d68fdcaf-faec-45e5-81f8-25da5c4626a8` (2026-06-29)

## Architect-pinned decisions (D1/D2/D3 — session ec9baef2)

The body marked this architect-led. Pass-1 returned READY with explicit decisions:

### D1 — Detection surface: Option B (`DelegationOutcome` enum)

Structured contract at the agent→team-engine boundary. Reason: Option A (tool_calls scan) is stringly-typed and brittle; Option C (sentinel strings) is fragile across model versions. The enum propagates typed information:

```rust
pub enum DelegationOutcome {
    Completed,
    TransportError { reason: String, retryable: bool },
    BusinessLogicFailure,    // model said "I can't" but got there cleanly
}
```

Populated by the agent loop's `run_silent_agent`/`run_team_agent` post-loop classifier — added as a new field on per-task records that survive into the team-engine `join_set` collection.

### D2 — Classifier scope: strict transport-only

Match against:
- HTTP 503 from `a2a_call` (founding incident shape)
- Substrings `transport error` | `connection refused` | `timeout` | `dns error` in `ToolOutput::error` output of any tool call made during the delegation
- `LoopResult::DeadlineExceeded` (hard timeout at agent loop layer)

NOT in scope (mika#1652 F1 invariant): business-logic apologies that arrive as `Ok(response)` — those correctly route to `TaskStatus::Completed` and `DelegationOutcome::BusinessLogicFailure`.

### D3 — Terminal state: new `RunStatus::FailedTransport` variant

Architect-confirmed: semantic clarity worth the enum extension cost. **Code-model note:** the current `RunStatus` enum (`crates/mika-agent/src/teams/types.rs:109`) is `{ Running (default), Completed, Suspended, Failed(String) }` — `Failed` is a **tuple variant carrying a String reason**, and the enum derives plain `serde::Serialize/Deserialize` (no `#[serde(rename)]` on any variant; DB string-form is the default PascalCase variant name via serde). The new variant should therefore mirror the existing shape as `FailedTransport(String)` (reason payload), not a unit variant — keeping symmetry with `Failed(String)` and preserving a human-readable reason for dashboards/metrics. The architect decides whether to add a `#[serde(rename)]` (would require touching the other variants for consistency) or accept the default `"FailedTransport"` string form. Reusing `RunStatus::Failed(String)` would blur the distinction between "all members transport-error (retryable in principle)" and "mixed/business-logic failure" — dashboards and metrics benefit from the distinction.

## Fix shape (skeleton — architect fills in via D1/D2/D3)

```
After the join_set collection loop finalizes all task statuses
(engine.rs:1287-1291, the post-loop "Running → Failed(panicked)" sweep),
and BEFORE the pending-grandchild suspend check (engine.rs:1293):
  1. For each task that completed, evaluate <D1 detection surface>.
  2. If ALL tasks in this iteration evaluate as terminal_transport_failure:
     → Set team_run.status = <D3 terminal state>
     → Emit TeamEvent::Failed { reason: "all-delegations-transport-failed" }
     → Skip review + deliver phases
     → Persist to DB + finalize_and_shutdown
  3. Else: proceed to the suspend check + review phase (current behavior).
```

**Ordering caveat (architect-bearing):** the suspend check at engine.rs:1293 returns early with `RunStatus::Suspended` when pending grandchild callbacks exist. An all-transport-failed iteration should still short-circuit rather than suspend — but the architect must confirm the interaction: if a member spawned a long-running grandchild before transport-failing, suspend-vs-fail precedence needs an explicit rule. The default proposal is: evaluate the all-transport-failed short-circuit FIRST (a fully transport-failed iteration has no useful grandchildren to wait on).

## Implementation outline (architect-shaped after pass-1)

1. **Architect first-pass decides D1/D2/D3 shape.** Plan revised in pass-2.
2. **Implement detection surface** per D1.
3. **Implement classifier** per D2 (with unit tests covering the ruleset).
4. **Wire short-circuit after engine.rs:1291** (post-status-finalization, pre-suspend-check) per D3.
5. **Replay test:** synthetic team-run with all members emitting 503 → assert run completes within seconds (not 30-min timeout), with the new terminal state.
6. **Regression test:** team-run with 1 transport-failed + 1 business-logic-completed (mixed) → assert NOT short-circuited (D2 invariant).

## Acceptance criteria

- **AC1** — Detection surface implemented per architect D1 decision. Code-cited reference to where the signal is read (tool_calls scan / DelegationOutcome enum / sentinel string).
- **AC2** — Classifier defined per D2 — explicit list of transport-class signals that qualify, with unit tests covering: 503 (qualifies), transport-error (qualifies), deadline-exceeded (qualifies), apologetic business response (does NOT qualify), success (does NOT qualify).
- **AC3** — Short-circuit fires when all completed delegations in an iteration qualify per AC2's classifier. Run transitions to terminal state without entering review/deliver.
- **AC4 (invariant guard)** — Mixed iteration (some transport-failed, some business-completed) does NOT short-circuit — proceeds to normal review/deliver flow. Regression test demonstrates.
- **AC5** — Replay: synthetic team-run reproducing founding incident `d68fdcaf-…` shape (all members emit 503) → run terminates within bounded latency (seconds, not minutes), team_run row marked with new terminal state.

## Out of scope

- **The reaper (mika#1652 AC1-3+AC5).** Already shipped. AC4 is a latency optimization on top.
- **Per-member retry-with-backoff before fail-counting.** Plan accepts the F1 invariant — if a delegation's agent loop completed with transport failure, that's signal. Retry logic lives at the agent loop layer, not the team engine.
- **a2a_call vs delegate_task tool-selection logic** — already fixed in mika#1653.

## Files involved

- `crates/mika-agent/src/teams/engine.rs:1242-1291` — join_set collection loop + post-loop status finalization; short-circuit insertion point is immediately after line 1291, before the pending-grandchild suspend check at line 1293
- `crates/mika-agent/src/teams/engine.rs:1718` — `parse_task_assignments` (referenced for context, NOT edited)
- `crates/mika-agent/src/teams/types.rs:109` — `RunStatus` enum (`Failed(String)` tuple variant); new `FailedTransport(String)` terminal variant (architect-bearing)
- `crates/mika-agent/src/agent_loop/mod.rs` — IF Option B chosen, post-loop classifier emits structured DelegationOutcome
- `crates/mika-agent/tests/teams/` — replay + regression tests

## Verification

- `cargo test -p mika-agent --test eval` — full eval matrix green
- `cargo test -p mika-agent teams::` — team engine tests including new AC4/AC5 fixtures
- Replay: synthetic team-run reproducing founding-incident shape terminates within bounded latency

## References

- mika#1652 — parent ticket; reaper shipped
- mika#1653 (CLOSED) — sibling tool-selection fix
- mika-arch first-pass session `6b2e7667` — the framing this plan corrects with codebase evidence
- Founding incident: `team-d68fdcaf-faec-45e5-81f8-25da5c4626a8` (Litha odds-engine, 2026-06-29, all members 503)
- `crates/mika-agent/src/teams/engine.rs:1242-1291` — result collection loop + status finalization (short-circuit target)
- `crates/mika-agent/src/teams/engine.rs:1718` — parse_task_assignments (mika#1652's incorrect target)
- `crates/mika-agent/src/tools/a2a_call.rs:146-153` — where a 503/transport failure becomes `ToolOutput::error` (the signal source)
- `crates/mika-agent/src/agent_loop/mod.rs::is_terminal_tool_error` — pattern that could be extended for transport classification
