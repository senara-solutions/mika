---
title: "fix: delegate_task should persist messages in delegate's session for observability"
type: fix
status: completed
date: 2026-03-24
issue: "#253"
---

# fix: delegate_task should persist messages in delegate's session for observability

## Overview

When the orchestrator delegates work via `delegate_task`, the delegate agent's session contains zero messages. The task prompt is injected into the system prompt (`TeamAgentParams.task_message`) and never written to the DB. The response flows back as `ToolOutput::success()` — also never persisted. This makes delegate interactions invisible in the dashboard, `unified_timeline` VIEW, and audit trail.

## Problem Statement

`delegate_task.rs` generates a `session_id` (UUID) but never creates a session row in the DB. `LoopMode::Team` has `saves_to_db() == false`, so `run_loop()` skips all message persistence. The team engine (`engine.rs`) works around this by manually persisting messages after `run_team_agent()` returns — but `delegate_task` does not.

**Evidence:** Session `472dd2c0` created for mika-qa during a QA delegation has zero message rows despite the agent returning VERDICT: PASS.

## Proposed Solution

Follow the team engine pattern (engine.rs lines 920-1042):

1. **Create session** in the DB before calling `run_team_agent()` — so tool-level DB writes (audit events) during the agent run have a valid session FK
2. **Persist "user" message** with the task text before the agent run
3. **Call `run_team_agent()`** (unchanged)
4. **Persist result message** — "assistant" for success, "system" for errors
5. **End session** before `async_db.shutdown()`

### Session Conventions

| Field | Value | Rationale |
|-------|-------|-----------|
| `session_id` | `delegate-{uuid}` (already generated) | Matches prefix convention; enables pruning |
| `channel_type` | `"system"` | Non-interactive, matches silent dispatcher convention |
| `metadata` | `{"trigger": "delegate", "orchestrator": "<agent_id>", "work_item_id": "<id>"}` | Dashboard context |
| `parent_session_id` | `ctx.session_id` (orchestrator's session) | Session hierarchy tracing |
| `trace_id` on messages | `ctx.trace_id` (orchestrator's) | Correlates in `unified_timeline` |

### Error Handling

- **Never** use `let _ =` on DB insert results (per P2-443 findings)
- Use `warn!` logging on persistence failures (per team engine pattern)
- Persistence failures do NOT affect the tool output — the delegate's response is still returned to the orchestrator
- Persist in all result paths: `Ok(Some(text))` → user + assistant, `Ok(None)` → user only (skip empty assistant), `Err(e)` → user + system error

### Pruning

Add `delegate-` prefix to `prune_old_sessions()` SQL in `db.rs` to prevent unbounded session accumulation (currently targets: `heartbeat-`, `callback-`, `skill-`, `reflection-`, `team-`).

## Technical Considerations

- **Agent registration:** `delegate_task.rs` does NOT call `register_agent()` for the delegate. The `sessions` table has FK to `agents(id)`. The team engine explicitly registers each agent before session operations (engine.rs line 95). Verify if the delegate agent is already registered at startup or add a `register_agent()` call. `create_session_if_not_exists` uses INSERT OR IGNORE which would still fail if the FK target doesn't exist.
- **Session creation timing:** BEFORE `run_team_agent()`, not after. Tools during the agent run reference `session_id` via `ToolContext`. Creating after would leave audit events referencing a non-existent session.
- **`async_db.shutdown()` ordering:** Shutdown must remain the last DB operation. Currently on line 243 — must move after all persistence calls.
- **`LoopMode::Team` unchanged:** We do NOT change `saves_to_db()`. The fix persists messages externally, matching the team engine pattern. Changing `saves_to_db()` would affect all team agent runs.

## Acceptance Criteria

- [x] Delegate session is created in the DB with correct metadata and parent linkage
- [x] "user" message with task text is persisted in the delegate's session
- [x] "assistant" message with response text is persisted on success
- [x] "system" message with error details is persisted on failure
- [x] Session is ended after the agent run
- [x] `prune_old_sessions()` includes `delegate-` prefix
- [x] Messages appear in `unified_timeline` with correct `trace_id`
- [x] Persistence failures are logged with `warn!` and do not affect tool output
- [x] `cargo test` passes (existing delegate_task validation tests still pass)
- [x] `cargo clippy` clean

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/tools/delegate_task.rs` | Session lifecycle + message persistence (primary change) |
| `crates/mika-agent/src/db.rs` | Add `delegate-` to `prune_old_sessions()` SQL |

## Sources

- Issue: [#253](https://github.com/senara-solutions/mika/issues/253)
- Team engine pattern: `crates/mika-agent/src/teams/engine.rs:920-1042`
- Silent dispatcher pattern: `crates/mika-agent/src/task_engine/dispatcher.rs:207-253`
- P2-443 error handling: `docs/solutions/logic-errors/team-engine-code-review-findings-batch.md`
- Trace ID linkage: `docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md`
