---
title: "Fix: Team delegations produce multiple callback messages instead of consolidated delivery"
type: fix
status: active
date: 2026-04-24
---

# Fix: Team delegations produce multiple callback messages instead of consolidated delivery

## Overview

A team run with N delegations produces N user-facing messages instead of one consolidated deliverable. The fix is a coupled two-part change to the task-engine's team callback plumbing: (1) add a single user-facing notification at team-run terminal status, sourced from the `deliverable` the team engine already produces — fired from both the synchronous return path (`run_team` tool) and the asynchronous resume path (`dispatch_invoke_orchestrator`), using a shared formatter helper; (2) suppress `send_message` in the per-child `resume_agent` silent turns so each delegation no longer independently notifies the user. Both parts must ship together — Part 1 alone produces N+1 messages (worse), Part 2 alone produces zero (silent).

No new schema, no API change, no new public types, no new engine-struct fields. Reuses the existing `NoopSender` in the dispatcher, the existing `message_sender` dispatch chain, and the existing `team_runs.deliverable` field.

## Problem Frame

When a team engine delegates to N agents in one run, it creates N child `resume_agent` callback tasks at `teams/engine.rs:874-912`. Each child's callback, when fired via `dispatch_resume_agent` at `task_engine/dispatcher.rs:295`, runs a full silent agent turn with `message_sender: self.message_sender.clone()` at line 393. The agent's turn independently calls `send_message`, producing one user-facing message per delegation.

The run-level deliverable path is split:
- **Sync completion** (all specialists finish inline, no suspension): `teams::run_team()` returns `TeamRun` with `status = "completed"` and `deliverable = Some(...)` to the `run_team` tool. The tool wraps it into `ToolOutput::success` for the orchestrator LLM (`tools/run_team.rs:144-151`). The user sees the deliverable only through the orchestrator's next turn, if the orchestrator decides to paraphrase it. No direct user-facing notification carries the deliverable text.
- **Async completion** (team suspends, then resumes via `invoke_orchestrator`): `dispatch_invoke_orchestrator` calls `resume_team_run` which drives the team engine's deliver phase. The tool has long since returned; the orchestrator LLM never gets a tool-output update. No user-facing notification is emitted anywhere. Today, async-completed team runs surface to the user **only** through per-child silent callbacks.

Meanwhile, `run_team` tool's `TeamEventCallback` (`tools/run_team.rs:97-126`) emits short progress announcements (`[Team] Phase: …`, `[Team] Agent 'alice' completed`, `[Team] Deliverable ready`, `[Team] Run failed: …`) to the orchestrator's channel during sync execution. On resume (async path), `new_for_resume` at `engine.rs:239` explicitly sets `callback: None` — so async runs produce no progress events either.

Observed failure: run `fd7ef7ef` (one concrete instance). mika-dev received 2 delegations → 2 child callbacks → 2 silent turns → 2 `send_message` calls → 2 user-visible messages in the TUI inbox.

## Requirements Trace

- **R1.** A completed team run must produce exactly one user-facing message containing the final deliverable text (not just a "ready" announcement), on both synchronous and asynchronous completion paths.
- **R2.** Per-delegation `resume_agent` callbacks must not call `send_message` on user-facing channels. The silent turn may still run (for internal state); only the user channel is gated.
- **R3.** Failure modes remain observable — on `status in (failed, cancelled)`, the user gets a single failure notification including `failure_reason` when set, not silence and not N partial-result messages.
- **R4.** The fix must not regress single-delegation team runs (N=1 → still one message, not zero) or conversational-reply team turns (orchestrator `{reply: "..."}` path unaffected).

## Scope Boundaries

### In scope

- `task_engine::dispatcher::dispatch_invoke_orchestrator` — add notification hook after `resume_team_run` (async path)
- `tools::run_team::execute` — add notification hook after `teams::run_team()` returns (sync path)
- A shared formatter helper in `teams::mod` or `teams::notification` for both callsites
- `task_engine::dispatcher::dispatch_resume_agent` — branch on team-child detection; pass `NoopSender` instead of `self.message_sender`
- Integration tests in `tests/eval/team_callback_consolidation.rs` covering N=1, N≥2, and failed team runs

### Deferred to Separate Tasks

- **Coverage check (#286):** different bug class
- **Removing `[Team] Deliverable ready` / `[Team] Run failed: …` from the existing `TeamEventCallback`:** these are now partially redundant with the new consolidated notification, but they fire *during* the run (not at completion) and are only present in the sync path. Removing them is a cleanup, not a fix. Out of scope; file as a follow-up if desired.
- **Populating `TeamEventCallback` on resume:** `new_for_resume` drops the callback — the new consolidated notification makes this less bad, but if we ever want progress events during resumed runs, that's a separate fix.
- **Orchestrator LLM paraphrasing of deliverable:** the orchestrator's next turn after a sync team run may send its own user message paraphrasing the deliverable. That's prompt-level behavior, not a plumbing bug. Out of scope.
- **Skipping the silent turn entirely for team-child callbacks:** potential optimization (no wasted LLM call per child). Defer; `NoopSender` is the minimal-footprint fix.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/teams/engine.rs:874-912` — child `resume_agent` callback tasks created per delegation with `team_run_id = Some(run_id)`, `parent_task_id = Some(parent_id)`, `trigger_type = "callback"`, `action_type = RESUME_AGENT`
- `crates/mika-agent/src/teams/engine.rs:451` — **only site** where `RunStatus::Failed(e.to_string())` is set. `failure_reason` is therefore always populated when `status == "failed"`.
- `crates/mika-agent/src/teams/engine.rs:507-544` — `finalize_and_shutdown()` converges sync and async paths on terminal state, but the team engine has no `user_message_sender` field — passing the notification through here would require rewiring the engine, which violates the "team engine's agents have `message_sender: None`" convention. The two-callsite approach at the engine's *callers* is cleaner.
- `crates/mika-agent/src/tools/run_team.rs:140-158` — sync path returns `TeamRun` to the tool; tool wraps into `ToolOutput::success` for the orchestrator LLM. Already has `ctx.message_sender` in scope via `ctx`. Natural seam for the sync notification.
- `crates/mika-agent/src/task_engine/dispatcher.rs:431-520` — `dispatch_invoke_orchestrator` handles async resume. Already has `self.message_sender` in scope. Natural seam for the async notification.
- `crates/mika-agent/src/task_engine/dispatcher.rs:295-425` — `dispatch_resume_agent`: per-callback silent turn with user-facing `message_sender` at line 393
- `crates/mika-agent/src/task_engine/dispatcher.rs:965` — existing `NoopSender` in the same module; reuse
- `crates/mika-agent/src/async_db.rs:494` — `count_pending_callback_tasks_by_team_run` already exists (sibling-completion detection is solved upstream)

### Institutional Learnings

- `docs/solutions/architecture-patterns/callback-resume-agent-lifecycle.md` — describes the resume_agent callback pattern this plan modifies
- `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md` — precedent for engine-level work in the callback path (metadata extraction runs BEFORE the silent turn); similar seam for suppressing the user-facing sender

## Key Technical Decisions

- **Two callsites, one shared formatter helper.** Sync path: in `run_team` tool, after `teams::run_team()` returns, format the deliverable/failure message and call `ctx.message_sender.send(...)`. Async path: in `dispatch_invoke_orchestrator`, after `resume_team_run` returns, load final `team_runs` row via `self.db.load_team_run_by_id(...)` and call `self.message_sender.send(...)`. **Rationale:** each path already has `message_sender` in scope at the right terminal-state boundary. Routing through `finalize_and_shutdown` would require adding a `user_message_sender` field to the team engine struct — cleaner looking from one angle, but it weakens the "team engine's agents don't talk to users directly" boundary. Duplicated *dispatch call* with a shared *formatter* is the right trade.
- **Shared helper: pure function, terminal-state only.** Lives at `teams::notification::build_run_completion_message(run: &TeamRun) -> Option<String>`. Returns `None` for non-terminal status (defensive; hook shouldn't fire in non-terminal paths anyway); returns `Some(text)` for `completed`/`failed`/`cancelled`. Trivial to unit-test without spinning up any engine harness.
- **Message format is committed (no "we'll pick later"):**

  | Status | Format |
  |---|---|
  | `completed` with deliverable | `"Team '{name}' completed. Deliverable:\n\n{deliverable_truncated}"` |
  | `completed` with `deliverable = None` (defensive) | `"Team '{name}' completed (no deliverable produced)."` + `warn!` |
  | `failed` | `"Team '{name}' failed: {failure_reason}"` — `failure_reason` is always present per `engine.rs:451` audit |
  | `cancelled` | `"Team '{name}' was cancelled."` |

  **Truncation:** deliverable longer than **4000 characters** (char count, not bytes — Unicode-safe boundary via `floor_char_boundary`) is cut and suffixed with `"\n\n[…truncated after 4000 chars — full deliverable persisted on team_runs.deliverable]"`. 4000 is below the Telegram 4096-char text limit with headroom for the prefix. The suffix explicitly names where the full text lives so the user isn't in the dark.

- **Suppress via `NoopSender`, not by skipping the silent turn.** The silent turn may still serve internal state (memory updates, task health awareness). Passing `Arc::new(NoopSender)` means `send_message` returns `Ok(Delivered)` without transmission. **Rationale:** minimal-footprint change; skipping the silent turn entirely is a larger behavioral change with unclear downstream effects (specialist memory/fact updates might rely on the silent turn firing). Defer that as a potential follow-up.
- **Detect "team-child callback" via `team_run_id.is_some() && parent_task_id.is_some()`.** Two-signal check; both columns are set together at child-creation time. Keep the helper `fn is_team_child_callback(task: &Task) -> bool` as a named predicate for intent clarity and test anchoring.
- **Observability via two locked structured log events:** `team_run_notified` (fires from both notification sites) and `team_child_callback_notification_suppressed` (fires in the `dispatch_resume_agent` branch). Schema below in the respective units.
- **`TeamEventCallback` untouched.** It fires short progress announcements during sync runs (`[Team] Phase: …`, etc.). Leaving it alone means sync runs produce: progress events during + deliverable at end. Async runs produce: deliverable at end only. The redundant `[Team] Deliverable ready` announcement is noted as out-of-scope cleanup.

## Open Questions

### Resolved During Planning

- **Sync path vs async path coverage:** Both paths need the hook. Resolved via two symmetric callsites + shared helper (above).
- **Message format:** Committed to the table above. No "decide later."
- **Failure path robustness:** `failure_reason` is always populated (audit: only `RunStatus::Failed(e.to_string())` at `engine.rs:451` ever sets failed state). The generic fallback case is unnecessary; dropped.
- **Can we skip the silent turn entirely for team-child callbacks?** Possible but larger blast radius. Deferred — `NoopSender` is the minimal change.
- **Should the consolidated message go through the orchestrator's conversation or be a system message?** System message. The deliverable is already produced by the team engine; no LLM turn needed.
- **Conversational-reply team turns (`{reply: "..."}`):** unaffected. That path produces the reply on the orchestrator's own channel (`engine.rs:596, 651` set `deliverable = reply`). Current behavior preserved; the new hook fires at deliverable time regardless.

### Deferred to Implementation

- **Exact module path for the shared helper.** `teams::notification::build_run_completion_message` is the leaning choice; finalize when creating the file.
- **Whether the sync path should skip the new notification when the returned `TeamRun` has `status == "suspended"`:** yes — only fire for terminal states. Helper returns `None` for non-terminal; sync callsite sends only on `Some`.

## Implementation Units

**Sequencing:** Both units land in the same PR as two commits. Unit 1 first (adds the consolidated hook; user temporarily sees N+1 messages on sync runs — do not deploy between commits). Commit 2 (Unit 2) suppresses per-child notifications; after it the user sees exactly 1 message per team run. Integration tests asserting the full flow land with Unit 2's commit.

---

- [x] **Unit 1: Shared formatter + notification hooks at sync and async terminal states**

**Goal:** On every team-run terminal transition, emit exactly one user-facing message containing the deliverable (or failure reason). Both sync completion (via `run_team` tool) and async completion (via `dispatch_invoke_orchestrator`) route through a shared pure formatter.

**Requirements:** R1, R3, R4

**Dependencies:** None

**Files:**
- Create: `crates/mika-agent/src/teams/notification.rs` — pure function `build_run_completion_message(run: &TeamRun) -> Option<String>` + inline `#[cfg(test)] mod tests`
- Modify: `crates/mika-agent/src/teams/mod.rs` — register new submodule, re-export helper
- Modify: `crates/mika-agent/src/tools/run_team.rs` — after `crate::teams::run_team(...)` returns, call the helper and send via `ctx.message_sender` if `Some(text)` returned
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs` — in `dispatch_invoke_orchestrator`, after `resume_team_run` returns, load final `team_runs` row, call the helper, send via `self.message_sender` if `Some(text)` returned

**Approach:**
- Helper branches on `run.status`:
  - `"completed"` with `run.deliverable == Some(d)` → format `"Team '{name}' completed. Deliverable:\n\n{d_truncated}"` with UTF-8-safe 4000-char truncation + suffix
  - `"completed"` with `run.deliverable == None` → `"Team '{name}' completed (no deliverable produced)."` + caller should `warn!`
  - `"failed"` → `"Team '{name}' failed: {failure_reason}"` (reason always present)
  - `"cancelled"` → `"Team '{name}' was cancelled."`
  - Anything else (`running`, `suspended`) → `None` (hook should not fire in these states; defensive)
- At both callsites, wrap the send in the existing `MessageSender` outcome handling: `Ok(Delivered)` → info log only; `Ok(NoChannel)` → silent (matches dispatcher policy elsewhere); `Ok(Failed)` → `warn!` and continue; `Err` → `warn!` and continue.
- **Sync callsite must Option-check `ctx.message_sender`.** Verified: `ToolContext.message_sender: Option<Arc<dyn MessageSender>>` (not every ctx construction site populates it — some test paths and the `toggle_skill` tool construct with `None`). On `None`: `debug!` log and skip the notification. Don't `warn!` — the absence is legitimate for some call paths, not a bug.
- **Async callsite reads `team_runs` via `self.db.load_team_run_by_id(...)`.** Verified: `resume_team_run` returns `Result<()>`, not `Result<TeamRun>` — the DB roundtrip is necessary. Single indexed PK read; negligible cost. Refactoring `resume_team_run` to return the final `TeamRun` would save the read but touches a larger API surface; out of scope for this fix (follow-up if anyone ever cares).
- **Anchor both callsites with a short code comment** so a future maintainer doesn't "helpfully" consolidate them into `TeamEngine::finalize_and_shutdown`:
  ```rust
  // Paired with <other callsite>; see docs/plans/2026-04-24-003-fix-team-callback-consolidation-plan.md
  // for why this lives here and not in TeamEngine::finalize_and_shutdown (keeps the team
  // engine free of a user_message_sender field).
  ```
- **Locked log schema** (observability contract — do not change field names without a plan update):

```rust
info!(
    team_run_id = %run.id,
    team_id = %run.team_id,
    team_name = %run.team_name,
    status = %run.status,
    notification_kind = %kind,       // "deliverable" | "failure" | "cancelled" | "fallback"
    deliverable_chars = run.deliverable.as_ref().map(|s| s.chars().count()).unwrap_or(0),
    truncated = truncated,           // bool: true if deliverable was cut to 4000 chars
    path = %path,                    // "sync" | "async"
    "team_run_notified"
);
```

- Grep pattern: `grep team_run_notified server.log | jq`.

**Patterns to follow:**
- Existing helper-function style in `teams::prompt::build_orchestrator_context` (pure, testable in-module)
- Existing `MessageSender` outcome handling in `server::verdict_handler` and `server::ci_success_handler`
- `UTF-8 safe char boundary` pattern at `teams/prompt.rs:206-213` (truncate at `is_char_boundary`)

**Test scenarios:**
- **Helper (happy, completed):** `TeamRun { status: "completed", team_name: "alpha", deliverable: Some("short text") }` → returns `Some("Team 'alpha' completed. Deliverable:\n\nshort text")`, `truncated = false`.
- **Helper (truncation):** deliverable is a 5000-char string → output is ≤ 4000 chars before suffix, suffix appended, cut on a UTF-8 char boundary (test with a multi-byte UTF-8 string near the boundary).
- **Helper (failed):** `status: "failed"`, `failure_reason: Some("orchestrator timed out")` → `Some("Team 'x' failed: orchestrator timed out")`.
- **Helper (cancelled):** `status: "cancelled"` → `Some("Team 'x' was cancelled.")`.
- **Helper (completed without deliverable):** `status: "completed"`, `deliverable: None` → `Some("Team 'x' completed (no deliverable produced).")`.
- **Helper (non-terminal):** `status: "running"` or `"suspended"` → `None`.
- **Sync hook (tool):** mock `ctx.message_sender`; `teams::run_team()` returns a completed `TeamRun` with deliverable. Assert: exactly one `send` call, text matches helper output, `team_run_notified` log with `path = "sync"`.
- **Sync hook (no message_sender):** `ctx.message_sender == None`. Assert: no send attempted, one `debug!` log emitted, tool return is unaffected.
- **Sync hook (non-terminal suspension):** `teams::run_team()` returns `status = "suspended"`. Assert: helper returns `None`, no send call.
- **Async hook (dispatcher):** `resume_team_run` returns success; `team_runs` table row has `status = "completed"`, `deliverable = Some(...)`. Assert: one send call, `team_run_notified` log with `path = "async"`.
- **Async hook (failed resume):** resume_team_run completes with `status = "failed"`. Assert: one send call with failure reason.
- **Async hook (NoChannel):** message_sender returns `Ok(NoChannel)`. Assert: no retry, no warn, dispatcher continues.

**Verification:**
- `cargo test -p mika-agent teams::notification` passes (helper unit tests).
- `cargo test -p mika-agent task_engine::dispatcher` passes (dispatcher hook tests).
- `cargo test -p mika-agent tools::run_team` passes (tool hook tests).

---

- [x] **Unit 2: Suppress `send_message` in per-child team-callback silent turns**

**Goal:** Per-delegation `resume_agent` callbacks run their silent turn with a `NoopSender` instead of the user-facing `message_sender`. With Unit 1 in place, the net result is exactly one user message per team run.

**Requirements:** R2, R4

**Dependencies:** Unit 1 (sequencing; both in same PR)

**Files:**
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs` — add private helper `fn is_team_child_callback(task: &Task) -> bool { task.team_run_id.is_some() && task.parent_task_id.is_some() }`; in `dispatch_resume_agent` near line 393, branch on the predicate and pass `Arc::new(NoopSender)` instead of `self.message_sender.clone()` when it returns `true`. Callsite comment explains *why* (see Minor Editorial).
- Test: inline unit test for `is_team_child_callback` (predicate truth table)
- Test: `crates/mika-agent/tests/eval/team_callback_consolidation.rs` (new) — end-to-end assertion that an N-delegation team run produces exactly 1 user-facing message

**Approach:**
- `NoopSender` already exists in the file (`dispatcher.rs:965`); reuse it.
- Log when suppression fires so the auditor can correlate with `team_run_notified`:

```rust
info!(
    task_id = %task.id,
    team_run_id = ?task.team_run_id,
    parent_task_id = ?task.parent_task_id,
    agent_id = %task.agent_id,
    "team_child_callback_notification_suppressed"
);
```

- No change to the silent turn itself — the turn still runs, still records `llm_calls`, still updates the specialist's memory if relevant. Only the user-facing channel is gated.
- Conversational-reply path is unaffected (no delegations → no children → no per-child callbacks).

**Patterns to follow:**
- Existing `NoopSender` usage at `dispatcher.rs:965`
- Existing structured logging conventions in the dispatcher

**Test scenarios:**
- **Predicate (team child):** `team_run_id = Some(...), parent_task_id = Some(...)` → `true`.
- **Predicate (non-team callback):** `team_run_id = None, parent_task_id = Some(...)` → `false` (regular skill callback with a parent).
- **Predicate (team root):** `team_run_id = Some(...), parent_task_id = None` — framed as an *invariant guard*: this shape isn't routed through `dispatch_resume_agent` today, but if someone later changes the team engine to route parents through this function, the predicate correctly returns `false`. Test catches the invariant violation if it slips.
- **Dispatcher branching:** with `is_team_child_callback == true`, `dispatch_resume_agent` constructs `SilentAgentParams` with `message_sender: Arc::new(NoopSender)`. With `false`, it uses `self.message_sender.clone()`. Asserted via a fake sender that records calls.
- **Integration (N=3 team run):** `MockLlmProvider` sequence drives a 3-delegation team run through to terminal status. Count `send_message` calls observed by the user-facing sender → exactly 1 (from Unit 1's hook). All 3 silent turns fire and record LLM calls; none hit the user channel.
- **Integration (N=1):** single-delegation team run → exactly 1 user message.
- **Integration (failed run):** team run fails during execute_tasks → exactly 1 user message containing the failure reason.

**Verification:**
- `cargo test -p mika-agent task_engine::dispatcher::tests::team_child_callback_predicate` passes.
- `cargo test -p mika-agent --test eval team_callback_consolidation` passes.
- Manual sanity: run a team with N ≥ 2 delegations in dev, observe exactly one message in the TUI inbox, and `grep team_run_notified server.log` finds one line per run while `grep team_child_callback_notification_suppressed` shows N suppression lines.

## System-Wide Impact

- **Interaction graph:** Two notification callsites (sync tool return, async dispatcher) plus one suppression branch in the callback dispatcher. Team engine internals and public APIs untouched.
- **Error propagation:** Both notification callsites wrap `message_sender` outcomes per existing dispatcher policy (`NoChannel` silent, `Failed` warn, `Err` warn).
- **State lifecycle risks:** None — silent turn still runs in Unit 2; internal specialist state unchanged. `team_runs.deliverable` is read, not written.
- **API surface parity:** No external API change. `run_team` tool, dashboard API, A2A endpoints unchanged.
- **Integration coverage:** Tests depend on a team-engine test harness that doesn't exist yet — see Risks.
- **Unchanged invariants:** `team_runs` schema, `tasks` schema, `MessageSender` trait, `SilentTrigger::Callback` semantics, team engine's inline specialist execution, per-specialist memory updates, `TeamEventCallback` progress events in the sync path.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Unit 1 and Unit 2 land in different commits; if deployed between them, UX regresses (N+1 messages on sync, 0 on async). | Both commits in one PR. PR body calls out "this fix is two coupled commits — do not cherry-pick one without the other." |
| **No existing team-engine test harness.** `tests/eval/*` targets `run_agent()`, not team runs. Integration tests require a small new harness scripted against the full team flow. | Shared risk with #286's Unit 2. Build the harness skeleton first. If it balloons past ~150 lines, narrow Unit 2's integration test to mock `dispatch_resume_agent` and `dispatch_invoke_orchestrator` calls directly (with crafted `Task`/`TeamRun` rows and a fake `message_sender`) and defer full-flow testing to a follow-up. Unit tests on the predicate and formatter are unaffected. If both #286 and #287 merge near each other, whichever lands first owns the harness; the other references it. |
| Orchestrator LLM's next turn after a sync team run may paraphrase the deliverable, producing a second user-visible message. | Out of scope — prompt-level behavior, not a plumbing bug. Document in the PR description; observe post-merge whether paraphrasing is actually happening and whether it's additive or confusing. Separate ticket if needed. |
| Sync path's existing `TeamEventCallback` still fires `[Team] Deliverable ready` — now redundant with the new consolidated notification. | Acceptable for this fix; the "ready" announcement is short and fires during the run, not at completion. Removing it is out-of-scope cleanup. File a follow-up ticket if the redundancy feels noisy in practice. |

## Documentation / Operational Notes

- Update `crates/mika-agent/CLAUDE.md` under the Task Engine section: *"Team-run user notification is fired once at terminal status from two symmetric callsites (`run_team` tool for sync completion, `dispatch_invoke_orchestrator` for async resume), both routing through `teams::notification::build_run_completion_message`. Per-child `resume_agent` callbacks have their user-facing `send_message` suppressed via `NoopSender`; the silent turn still runs — it updates memory and records `llm_calls` — only the user channel is gated."*
- No deployment, migration, or rollout concerns.
- **Pre-existing hardening (optional, not required by this fix):** add a one-line test asserting `NoopSender` returns `Ok(SendOutcome::Delivered)`. If its semantics ever drift (e.g., someone adds a warn log), Unit 2's suppression log semantics would silently change. Include only if trivial; don't block the PR on it.

## Sources & References

- **Origin issue:** [senara-solutions/mika#287](https://github.com/senara-solutions/mika/issues/287)
- Related code: `crates/mika-agent/src/teams/engine.rs`, `crates/mika-agent/src/task_engine/dispatcher.rs`, `crates/mika-agent/src/tools/run_team.rs`
- Related pattern: `docs/solutions/architecture-patterns/callback-resume-agent-lifecycle.md`
- Related pattern: `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md`
- Reuse: `dispatcher.rs:965` — existing `NoopSender`
- `failure_reason` audit: `engine.rs:451` is the sole `RunStatus::Failed` callsite, always passes `e.to_string()`; reason is always populated
- Observed failure: team run `fd7ef7ef` — one concrete instance of the contract gap
