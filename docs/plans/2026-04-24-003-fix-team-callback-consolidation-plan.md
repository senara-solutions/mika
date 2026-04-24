---
title: "Fix: Team delegations produce multiple callback messages instead of consolidated delivery"
type: fix
status: active
date: 2026-04-24
---

# Fix: Team delegations produce multiple callback messages instead of consolidated delivery

## Overview

A team run with N delegations produces N user-facing messages instead of one consolidated deliverable. The fix is a coupled two-part change to the task-engine's team callback plumbing: (1) add a single user-facing notification at team-run completion, sourced from the `deliverable` the team engine already produces; (2) suppress `send_message` in the per-child `resume_agent` silent turns so each delegation no longer independently notifies the user. Both parts must ship together — Part 1 alone produces N+1 messages (worse), Part 2 alone produces zero (silent).

No new schema, no API change, no new public types. Both hooks live inside `task_engine::dispatcher` and reuse the existing `message_sender` wired into the dispatcher.

## Problem Frame

When a team engine delegates to multiple agents in one run, it creates N child `resume_agent` callback tasks (`teams/engine.rs:874-912`). Each child's callback, when fired via `dispatch_resume_agent` (`task_engine/dispatcher.rs:295`), runs a full silent agent turn with `message_sender: self.message_sender.clone()` (line 393). The agent's turn then independently calls `send_message`, producing one user-facing message per delegation.

The parent `invoke_orchestrator` task (`dispatcher.rs:431`) fires once all children complete. It resumes the team engine via `resume_team_run`, which continues to the review/deliver phase and produces `team_runs.deliverable`. But it **does not** emit a user-facing message — the only user-visible path for team output today is the per-child silent turns.

Observed: run `fd7ef7ef` (one concrete instance), mika-dev received 2 delegations → 2 callback tasks → 2 silent turns → 2 `send_message` calls → 2 user-visible messages in the TUI inbox.

## Requirements Trace

- **R1.** A completed team run must produce exactly one user-facing message containing the final deliverable.
- **R2.** Per-delegation callbacks must not call `send_message` for user-facing channels. They may still run the silent turn for internal state if needed, or be short-circuited entirely — but the user-facing channel is closed to them.
- **R3.** Failure modes remain observable — if the team run fails (timeout, orchestrator error), the user gets a single failure notification with the reason, not silence and not N partial-result messages.
- **R4.** The fix must not regress single-delegation team runs (N=1 → still one message, not zero) or conversational-reply team turns (no delegation, no notification expected unless the orchestrator itself replied).

## Scope Boundaries

### In scope

- `task_engine::dispatcher::dispatch_invoke_orchestrator` — add user-notification hook at completion
- `task_engine::dispatcher::dispatch_resume_agent` — detect team-child callbacks and pass `NoopSender` (already imported at dispatcher.rs:965 for another path) instead of the user-facing sender
- Synchronous team-run completion path (when all specialists finish inline without suspending) — add the same notification hook
- Integration tests in `tests/eval/` covering 1-delegation, N-delegation, and failed-team-run scenarios

### Deferred to Separate Tasks

- **Coverage check (#286):** different bug class, separate plan
- **Dashboard team-run surfacing:** the dashboard already displays the `deliverable` field; no new surfacing needed for this fix
- **Message formatting customization:** the consolidated message uses the raw `deliverable` text (possibly with a short header). Richer formatting is a UX concern, not a bug fix
- **Per-agent "I delegated this to you, here's the outcome" internal messages:** if product wants the orchestrator to personally notify each specialist's conversation thread about the team run, that's a separate feature — this plan only addresses the user-facing channel

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/teams/engine.rs:874-912` — child `resume_agent` callback tasks created per delegation with `team_run_id = Some(run_id)`, `parent_task_id = Some(parent_id)`, `trigger_type = "callback"`, `action_type = RESUME_AGENT`
- `crates/mika-agent/src/teams/engine.rs:838-866` — parent `invoke_orchestrator` task with `action_type = INVOKE_ORCHESTRATOR`, fires when all children complete
- `crates/mika-agent/src/teams/engine.rs:486-505` — `deliver_phase()` sets `self.run.deliverable` and emits `TeamEvent::Deliverable(...)` for live UI; does NOT send user message
- `crates/mika-agent/src/task_engine/dispatcher.rs:295-425` — `dispatch_resume_agent`: per-callback silent turn, passes `message_sender: self.message_sender.clone()` at line 393
- `crates/mika-agent/src/task_engine/dispatcher.rs:431-520` — `dispatch_invoke_orchestrator`: calls `resume_team_run`, no user notification
- `crates/mika-agent/src/task_engine/dispatcher.rs:965` — `NoopSender` already exists in the file for another path; reuse it
- `crates/mika-agent/src/async_db.rs:494` — `count_pending_callback_tasks_by_team_run` already exists for sibling-completion detection
- `crates/mika-agent/CLAUDE.md` → MessageSender section: team engine agents intentionally have `message_sender: None`; this plan extends that discipline to per-child callback silent turns

### Institutional Learnings

- `docs/solutions/architecture-patterns/callback-resume-agent-lifecycle.md` — describes the resume_agent callback pattern this plan modifies
- `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md` — precedent for engine-level work in the callback path (metadata extraction runs BEFORE the silent turn); similar seam for suppressing the user-facing sender
- `docs/solutions/architecture-patterns/generic-callback-framing-parent-task-id.md` — parent_task_id is already a first-class signal in the callback path

### Unknowns to resolve during implementation

- **Where exactly to hook the consolidated notification.** Candidates: (a) inside `dispatch_invoke_orchestrator` after `resume_team_run` returns, loading final `team_runs.deliverable`; (b) inside `resume_team_run` itself near the deliver-phase completion; (c) inside the `run_team` tool's caller for synchronous completion. Must handle both sync and async paths. Leaning toward (a) for async + a matching hook in the sync return path — resolved once the hooks are in place.
- **Failure-path notification shape.** Existing `team_runs.status` values: `running`, `completed`, `failed`, `cancelled`, `suspended`. Plan: on `failed`/`cancelled`, send a short status line + failure_reason if present. On `completed`, send the deliverable.

## Key Technical Decisions

- **The two parts are coupled and must ship together.** Part 1 (add consolidated notification) alone = N+1 messages. Part 2 (suppress per-child) alone = zero messages. Both in the same PR, landing as sequenced commits so the diff is legible.
- **Suppress via `NoopSender`, not by skipping the silent turn.** The silent turn may still serve internal state (memory updates, task health awareness per the CLAUDE.md "Silent Mode Agent Loop" section). Passing a `NoopSender` instead of `None` means `send_message` becomes a no-op without triggering the channel-routing code paths (which have their own warn-log side effects via `NoChannel`). **Rationale:** this is a minimal-footprint change at one dispatcher seam. Skipping the silent turn entirely is a larger behavioral change with unclear downstream effects (e.g., internal state the specialist might rely on next turn); defer that as a potential follow-up if the silent turn proves wasteful.
- **Detect "team-child callback" via `team_run_id.is_some() && parent_task_id.is_some()`.** These are set together at child-creation time (engine.rs:877-878). A task with a team_run_id but no parent_task_id would be the parent itself (invoke_orchestrator); its path is different and not affected. **Rationale:** two-signal check avoids false-negatives if any other code path populates one but not the other; explicit is better than clever.
- **Consolidated message is sent from `dispatch_invoke_orchestrator` after completion.** This hook sees the team run transitioning to `completed`/`failed`/`cancelled` after `resume_team_run` returns. It's the single convergence point for both async-suspended runs and post-review deliveries. **Rationale:** matches where `team_runs.deliverable` is already committed to the DB; avoids duplicating the hook in the synchronous path inside the team engine.
- **Synchronous completion path also needs a hook.** If all specialists finish without suspending, the team engine completes inline inside `execute()` and never goes through `invoke_orchestrator`. The `run_team` tool is the caller in that case — it already returns the deliverable to the orchestrator conversation. Evaluate whether the per-child silent callbacks even fire in that path; if they do, the Part-1 hook needs a sibling inside the tool's completion handling. Resolved during implementation — a small investigation before Part 1 writes the code.
- **No new schema, no API change.** `team_runs.deliverable` already exists. `message_sender` dispatch path already exists. `NoopSender` already exists. This is purely rewiring two seams.
- **Observability via structured `info!`/`warn!` logs.** Log fields: `team_run_id`, `team_id`, `delegation_count`, `notification_kind` (`"deliverable"` | `"failure"` | `"suppressed_per_child"`). Locked schema below under Unit 1.

## Open Questions

### Resolved During Planning

- **Can we skip the silent turn entirely for team-child callbacks?** Possible but larger blast radius. Deferred — NoopSender is the minimal change.
- **Should the consolidated message go through the orchestrator's conversation or be a system message?** System message (same channel the current per-child silent turns use). The deliverable is already produced by the team engine; no LLM turn is needed to format it.
- **What about the case where the orchestrator emits `{reply: "..."}` instead of delegating?** That path produces a conversational reply on the orchestrator's own channel (engine.rs:596, 651 set `deliverable = reply`). Current behavior is preserved — the consolidated-message hook fires at deliverable time regardless of whether delegations happened.

### Deferred to Implementation

- **Exact message format for the consolidated notification.** A simple `"Team '{name}' finished: {deliverable}"` or just the raw deliverable. Will pick when writing Unit 1; low-risk.
- **Whether to also suppress the silent turn's entry in `llm_calls` table.** Probably keep — `llm_calls` is observability, not user-facing. Silent turn still happens, LLM call still recorded, just `send_message` is a no-op.

## Implementation Units

**Sequencing:** Both units land in the same PR as two commits. Unit 1 first (adds the consolidated hook, but user temporarily sees N+1 messages — do not deploy after commit 1). Commit 2 (Unit 2) suppresses per-child notifications; after it the user sees exactly 1 message per team run. Integration test for the full flow lives with Unit 2's commit.

- [ ] **Unit 1: Consolidated team-run notification hook**

**Goal:** When a team run reaches terminal status (`completed`/`failed`/`cancelled`), send exactly one user-facing message via `self.message_sender` on the dispatcher.

**Requirements:** R1, R3, R4

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs` — extend `dispatch_invoke_orchestrator` to notify on terminal status after `resume_team_run`
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs` — **investigation step first**: confirm whether the synchronous completion path (team engine finishes inline in `execute()` without suspending) fires per-child callbacks. If yes, add the sibling hook; if no, document why only the async path needs one
- Test: `crates/mika-agent/src/task_engine/dispatcher.rs#tests` — unit test for the notification formatter
- Test: `crates/mika-agent/tests/eval/team_callback_consolidation.rs` (new) — integration test; see harness risk in Risks table

**Approach:**
- After `resume_team_run` returns in `dispatch_invoke_orchestrator`, load the final `team_runs` row via `self.db.load_team_run_by_id(team_run_id)`.
- Build the message per this logic:
  - `status == "completed"` AND `deliverable.is_some()` → `deliverable` text (truncated to a safe limit, TBD during implementation)
  - `status == "failed"` OR `status == "cancelled"` → `"Team '{team_name}' {status}{optional_failure_reason}"`
  - Anything else (unexpected status transition) → log `warn!` and emit a generic fallback
- Call `self.message_sender.send(...)` with the standard `message` + `chat_id` resolution already used elsewhere in the dispatcher. Respect `Ok(SendOutcome::NoChannel)` (agent has no reply channel — do not warn or retry; matches existing dispatcher policy).
- Emit a structured log line documenting what was sent. Locked schema:

```rust
info!(
    team_run_id = %run.id,
    team_id = %run.team_id,
    team_name = %run.team_name,
    status = %run.status,
    delegation_count = children.len(),
    notification_kind = %kind,  // "deliverable" | "failure" | "fallback"
    deliverable_chars = run.deliverable.as_ref().map(|s| s.chars().count()).unwrap_or(0),
    "team_run_notified"
);
```

- Grep pattern: `grep team_run_notified server.log | jq`.

**Patterns to follow:**
- Existing `info!`/`warn!` structured logging conventions in the dispatcher
- Existing `message_sender` dispatch call sites (look at how `verdict_handler` and `ci_success_handler` send notifications)
- `MessageSender` trait's three outcomes (`Delivered`/`Failed`/`NoChannel`) handled per existing policy in this file

**Test scenarios:**
- **Happy path (completed + deliverable):** team_runs row has `status="completed"`, `deliverable=Some("...")`. Test: one `send_message` call issued, message contains the deliverable text, `team_run_notified` log emitted with `notification_kind="deliverable"`.
- **Happy path (failed):** `status="failed"`, `failure_reason=Some("orchestrator timeout")`. Test: one call, message contains both the team name and the reason; log has `notification_kind="failure"`.
- **Edge case (completed, no deliverable):** `status="completed"`, `deliverable=None` (shouldn't happen per team engine contract, but defensive). Test: `notification_kind="fallback"`, message is a generic status line, `warn!` emitted.
- **Edge case (NoChannel):** `message_sender.send()` returns `Ok(NoChannel)`. Test: no retry, no warn, function returns `Ok(())`.
- **Edge case (unexpected status):** `status="suspended"` at this point would be unexpected. Test: `warn!` emitted, no user-facing message (defer to the next fire of `invoke_orchestrator`).

**Verification:**
- `cargo test -p mika-agent task_engine::dispatcher` passes.
- Manual sanity post-integration: after a team run completes in dev, `grep team_run_notified server.log | jq '.fields'` shows the event with expected fields.

---

- [ ] **Unit 2: Suppress `send_message` in per-child team-callback silent turns**

**Goal:** Per-delegation `resume_agent` callbacks run their silent turn with a `NoopSender` instead of the user-facing `message_sender`, so `send_message` becomes a no-op. With Unit 1 in place, the net result is exactly one user message per team run.

**Requirements:** R2, R4

**Dependencies:** Unit 1 (sequencing; both in same PR)

**Files:**
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs` — at `dispatch_resume_agent:393`, branch on `is_team_child_callback(task)` and pass `Arc::new(NoopSender)` in that branch instead of `self.message_sender.clone()`
- Add: `crates/mika-agent/src/task_engine/dispatcher.rs` — small private helper `fn is_team_child_callback(task: &Task) -> bool { task.team_run_id.is_some() && task.parent_task_id.is_some() }`
- Test: inline unit test for `is_team_child_callback` (predicate truth table)
- Test: `crates/mika-agent/tests/eval/team_callback_consolidation.rs` — end-to-end assertion that an N-delegation team run produces exactly 1 user-facing message

**Approach:**
- `NoopSender` already exists in the file (dispatcher.rs:965); it returns `Ok(SendOutcome::Delivered)` without actually sending.
- Log when suppression fires so we can audit:

```rust
info!(
    task_id = %task.id,
    team_run_id = ?task.team_run_id,
    agent_id = %task.agent_id,
    "team_child_callback_notification_suppressed"
);
```

- No change to the silent turn itself — the turn still runs, still records `llm_calls`, still updates the specialist's memory as needed. Only the user-facing channel is gated.
- Conversational-reply path is unaffected — no delegations means no children, means no per-child callbacks fire at all.

**Patterns to follow:**
- The existing `NoopSender` usage at dispatcher.rs:965 for its other path — mirror the instantiation pattern.
- Existing structured logging conventions in the dispatcher.

**Test scenarios:**
- **Happy path (team child):** task with `team_run_id=Some(...)`, `parent_task_id=Some(...)` → `is_team_child_callback` returns `true`, `NoopSender` is passed to the silent turn, suppression log emitted.
- **Happy path (non-team callback):** task with `team_run_id=None`, `parent_task_id=Some(...)` (e.g. a regular skill callback with a parent) → `is_team_child_callback` returns `false`, user-facing `message_sender` is passed as today.
- **Edge case (team-root task):** task with `team_run_id=Some(...)`, `parent_task_id=None` (the `invoke_orchestrator` parent itself). Not routed through `dispatch_resume_agent` but defensive unit test ensures `is_team_child_callback` returns `false`.
- **Integration (end-to-end):** 3-delegation team run via `MockLlmProvider`. Count `send_message` calls observed by the dispatcher's message_sender — exactly 1 (from Unit 1's consolidated hook). All 3 silent turns fire and record LLM calls, but none call user-facing `send_message`.
- **Integration (single delegation):** 1-delegation team run → 1 message.
- **Integration (failed run):** team run fails mid-execution → 1 message with failure reason.

**Verification:**
- `cargo test -p mika-agent task_engine::dispatcher::tests::team_child_callback_predicate` passes.
- `cargo test -p mika-agent --test eval team_callback_consolidation` passes.
- Manual sanity: run a team with N ≥ 2 delegations, observe exactly one message in the TUI inbox, and `grep team_run_notified server.log` finds one line per run.

## System-Wide Impact

- **Interaction graph:** Two seams inside `task_engine::dispatcher`. No changes to team engine internals or to public task/team APIs.
- **Error propagation:** Unit 1's notification wraps `message_sender` failures per existing policy (warn on `Failed`, silent on `NoChannel`, `Err` logged and swallowed to not block the dispatcher loop).
- **State lifecycle risks:** None — the silent turn still runs in Unit 2, so internal specialist state is unchanged. Only the external user-channel call becomes a no-op.
- **API surface parity:** No external API change. `run_team` tool, dashboard API, A2A endpoints unchanged.
- **Integration coverage:** Tests depend on a team-engine test harness that doesn't exist yet (see Risks). Unit-level tests on the predicate and notification formatter are unaffected.
- **Unchanged invariants:** `team_runs` schema, `tasks` schema, `message_sender` trait, `SilentTrigger::Callback` semantics, team engine's inline specialist execution, per-specialist memory updates.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Unit 1 and Unit 2 land in different commits; if deployed between them, UX regresses (N+1 or 0 messages). | Commit sequencing lives inside one PR; ship atomically. PR description calls out "this fix is two coupled commits — do not cherry-pick one without the other." |
| Synchronous completion path (all specialists finish inline without suspending) may not go through `dispatch_invoke_orchestrator`; the consolidated notification would miss those runs. | Unit 1 begins with a small investigation step. If sync path is unaffected (because per-child callbacks don't fire in that path either), no sibling hook needed; if per-child callbacks DO fire, add the hook in the `run_team` tool's return path. Outcome documented in the PR description regardless. |
| **No existing team-engine test harness.** Same risk as #286's Unit 2 — `tests/eval/*` targets `run_agent()`, not team runs. Integration tests require a small new harness scripted against the full team flow. | Build the harness skeleton first. If it balloons, narrow Unit 2's integration test to mock `dispatch_resume_agent` and `dispatch_invoke_orchestrator` calls directly (with crafted `Task` rows and a fake `message_sender`), and defer full-flow testing to a follow-up. Unit-level tests on the predicate and formatter remain unaffected. Consider coordinating this harness with #286's — if both tickets land the same harness, either plan should reference the other in its PR. |
| `NoopSender` semantics drift — if someone later changes it to log a warning or record telemetry, Unit 2's log dedupe breaks. | `NoopSender` is in the dispatcher module; add a test that asserts its current `Delivered` no-op behavior so future changes trip the test. One-line regression guard. |
| Suppression log becomes noisy under high delegation volume. | `info!` not `warn!`. Can be filtered out downstream. The benefit — being able to audit "was the suppression active for this run?" — outweighs the volume cost. |
| Team-run failure with `deliverable = None` produces a fallback message, but the team engine may not always set `failure_reason`. | Unit 1's fallback case covers this: emits a generic status line + `warn!`. Not user-hostile, but we note in the plan that populating `failure_reason` at team-engine failure points is a follow-up cleanliness fix (out of scope). |

## Documentation / Operational Notes

- Update `crates/mika-agent/CLAUDE.md` under the Task Engine section with a short note: "Team-run user notification is fired once at terminal status from `dispatch_invoke_orchestrator`; per-child `resume_agent` callbacks have their user-facing `send_message` suppressed via `NoopSender`."
- No deployment, migration, or rollout concerns — purely an internal rewiring.

## Sources & References

- **Origin issue:** [senara-solutions/mika#287](https://github.com/senara-solutions/mika/issues/287)
- Related code: `crates/mika-agent/src/teams/engine.rs`, `crates/mika-agent/src/task_engine/dispatcher.rs`
- Related pattern: `docs/solutions/architecture-patterns/callback-resume-agent-lifecycle.md`
- Related pattern: `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md`
- Related code: `dispatcher.rs:965` — existing `NoopSender` reuse
- Observed failure: team run `fd7ef7ef` — one concrete instance, not a provider-specific conclusion
