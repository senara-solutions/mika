---
title: "Team delegations produce multiple callback messages instead of consolidated delivery"
date: 2026-04-24
category: logic-errors
module: mika-agent — task_engine/dispatcher, teams/engine, tools/run_team
problem_type: logic_error
component: tooling
symptoms:
  - "User receives N separate system messages for a single team run with N delegations"
  - "Multiple callback tasks created per team run, each triggering independent send_message calls"
  - "TUI inbox shows duplicate events for the same team run completion"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags:
  - team-engine
  - callback
  - notification
  - message-sender
  - noop-sender
  - dispatch-resume-agent
  - consolidated-delivery
---

# Team delegations produce multiple callback messages instead of consolidated delivery

## Problem

When a team agent receives multiple delegations during a single team run, each delegation completion triggers a separate callback task that calls `send_message` independently. The user receives N separate system messages instead of a single consolidated team result notification. Observed in team run `fd7ef7ef` where mika-dev received 2 delegations, creating 2 callback tasks that each produced a separate user-visible message.

## Symptoms

- User sees N system events in the TUI inbox for a single team run with N delegations
- Each `resume_agent` callback task independently fires `send_message` via the user-facing `message_sender`
- No consolidated deliverable notification — only per-agent fragments

## What Didn't Work

- **Routing through `TeamEngine::finalize_and_shutdown`**: Would require adding a `user_message_sender` field to the team engine struct, violating the architectural boundary that "team engine's agents don't talk to users directly." The team engine intentionally has `message_sender: None`.
- **Skipping the silent turn entirely for team-child callbacks**: Larger behavioral change with unclear downstream effects — specialist memory/fact updates might rely on the silent turn firing.
- **Single callback per run (removing per-child callbacks)**: The per-child callbacks serve internal state purposes (memory updates, `llm_calls` recording). Removing them entirely risks breaking specialist internal state.

## Solution

Two coupled changes that must ship together (Part 1 alone = N+1 messages; Part 2 alone = 0 messages):

**Part 1: Shared formatter + notification hooks at terminal state**

Created `teams::notification` module with a pure formatter function that builds the user-facing message from a team run's terminal state:

```rust
// teams/notification.rs
pub(crate) fn build_run_completion_message(run: &TeamRun) -> Option<CompletionMessage> {
    match &run.status {
        RunStatus::Completed => { /* format deliverable with 4000-char UTF-8-safe truncation */ }
        RunStatus::Failed(reason) => { /* format failure reason */ }
        RunStatus::Running | RunStatus::Suspended => None, // non-terminal, no notification
    }
}
```

Two symmetric callsites invoke this helper:
- **Sync path** (`tools/run_team.rs`): after `teams::run_team()` returns, sends via `ctx.message_sender`
- **Async path** (`task_engine/dispatcher.rs`): after `resume_team_run` returns, loads final `team_runs` row via `self.db.load_team_run_by_id()`, sends via `self.message_sender`

**Part 2: Suppress per-child `send_message` via `NoopSender`**

```rust
// dispatcher.rs — dispatch_resume_agent
let message_sender = if is_team_child_callback(task) {
    Some(Arc::new(NoopSender) as Arc<dyn MessageSender>)
} else {
    self.message_sender.clone()
};
```

`is_team_child_callback` detects team-child callbacks via `task.team_run_id.is_some() && task.parent_task_id.is_some()` — both fields are set together at child-creation time in `engine.rs:874-912`.

`NoopSender` (promoted to `pub` in `messaging.rs`) returns `Ok(SendOutcome::Delivered)` without transmission. The silent turn still runs — memory updates, `llm_calls` recording, and internal state all proceed normally. Only the user-facing channel is gated.

## Why This Works

The root cause is that each per-delegation `resume_agent` callback creates a full silent agent turn with the user-facing `message_sender`, and the agent's turn independently calls `send_message`. The team run already produces a consolidated `deliverable` field — but no code path was sending it to the user as a single notification.

The fix adds the missing consolidated notification at the two terminal-state boundaries (sync and async), then suppresses the redundant per-child user channel. The `NoopSender` approach is minimal-footprint — it doesn't change the silent turn behavior at all, only the `send_message` outcome.

The dual-callsite design preserves the architectural boundary that the team engine itself has no `user_message_sender` field. Each callsite lives at the *caller* of the team engine, where `message_sender` is already in scope.

## Prevention

- **Structural guard for one-consumer-per-channel**: When multiple code paths can send messages to the same user channel from the same logical operation, use a structural suppressor (`NoopSender`, `cli_mode` guard) rather than relying on ordering or timing. This mirrors the pattern from `callback-processing-race-steals-tui-notifications.md`.
- **Observability contract**: Two locked structured log events (`team_run_notified` with `path=sync|async`, `team_child_callback_notification_suppressed`) allow operators to verify exactly-one delivery per team run via `grep team_run_notified server.log | jq`.
- **Paired changes**: The plan explicitly documents that Parts 1 and 2 must ship together in one PR. The PR body warns against cherry-picking one without the other.

## Related Issues

- [senara-solutions/mika#287](https://github.com/senara-solutions/mika/issues/287) — origin issue
- `docs/solutions/logic-errors/callback-processing-race-steals-tui-notifications.md` — same structural pattern (one-consumer guard via mode flag)
- `docs/solutions/architecture-patterns/callback-resume-agent-lifecycle.md` — callback/resume lifecycle this fix modifies
- `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md` — precedent for engine-level work in the callback path
