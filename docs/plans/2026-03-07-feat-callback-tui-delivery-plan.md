---
title: "feat: Callback TUI Delivery"
type: feat
status: completed
date: 2026-03-07
origin: docs/brainstorms/2026-03-07-callback-tui-delivery-brainstorm.md
---

# feat: Callback TUI Delivery

## Overview

Make long-running task callback results visible in the TUI chat conversation. Currently, when a background task (e.g., `analyze_codebase`) completes, the result is stored in the DB but the TUI never updates — the user sees "analysis running" as the last message forever. This closes the gap between the expected UX (described in the unified-task-engine brainstorm) and the actual implementation.

## Problem Statement / Motivation

The callback delivery chain breaks at three points in CLI/TUI mode:

1. **No TUI connection from silent agent:** `dispatch_resume_agent` runs a silent agent with no `MessageSender`, so the agent can't deliver results to the TUI. The agent itself notes "no outbound messaging channel configured" and gives up.

2. **TUI doesn't poll callback messages:** `POLLED_CHANNELS = ["telegram", "cli"]` in `app.rs:335` excludes `"callback"` channel. Even if messages were saved, the TUI wouldn't display them.

3. **Footer ignores callback tasks:** `get_pending_reminder_tasks()` filters for `trigger_type IN ('time', 'recurring')` only. Pending callback tasks are invisible in the footer.

Server mode (Telegram) works because the dispatcher passes a `GatewayMessageSender` and the silent agent calls `send_message`. That path stays unchanged.

## Proposed Solution

Replace the silent-agent callback path in CLI/TUI mode with a **TUI-injected conversation turn**:

1. `mika ask --task-id` marks the task `completed` in DB and exits (no silent agent run)
2. TUI tick loop polls for completed-but-undelivered callback tasks every N seconds
3. When detected, TUI sends `AgentRequest::CallbackResult` through the agent worker channel
4. Agent worker runs a **normal `run_agent()` turn** with the callback result as injected context
5. Agent processes results and responds — user sees both the result and the response live
6. Task is marked `delivered` after successful processing

(See brainstorm: `docs/brainstorms/2026-03-07-callback-tui-delivery-brainstorm.md` — Decision 1)

## Technical Considerations

### Schema Migrations (v1 → v2)

SQLite does not support `ALTER TABLE ... ALTER COLUMN` or modifying CHECK constraints in place. Both the `delivered` status and `tool_result` role require the standard SQLite table recreation pattern:

```sql
-- Pattern: create new → copy → drop old → rename (within a transaction)
BEGIN;
CREATE TABLE tasks_new (... CHECK (status IN (..., 'delivered')));
INSERT INTO tasks_new SELECT * FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;
-- Recreate all indexes
COMMIT;
```

Same pattern for `conversations` table (`tool_result` role).

**Migration function:** `migrate_v2()` in `db.rs`, called when `schema_version < 2`.

**Terminal status queries:** All code that checks for terminal statuses (e.g., `try_complete_parent_on_sibling_done` checking `completed/failed/expired/cancelled`) must also include `delivered`. Grep for these patterns and update them.

**Pruning:** `prune_old_tasks()` must include `delivered` alongside `completed/failed/cancelled` to prevent accumulation.

### Loop Prevention (Defense in Depth)

(See brainstorm: Decision 3)

**Code guard:** Pass `long_running_ctx = None` to the agent loop during callback turns (`agent.rs:786`). This is the same pattern already used by the silent agent loop (`agent.rs:1354`). The agent literally cannot create callback tasks because `execute_skill_tool` requires a `LongRunningContext` to spawn background processes.

**Prompt guard:** Inject into system prompt during callback turns:
```
IMPORTANT: This is a callback result turn. You MUST NOT submit new long-running tasks.
Process the results and respond directly.
```

### Agent Busy Handling

Callback polling is guarded by `AgentStatus::Idle` (same pattern as cross-channel polling in `app.rs`). When the agent is busy processing a user message, the completed task stays in `completed` status in DB. The next idle poll cycle picks it up. No callbacks are dropped — they just wait.

### Multiple Callbacks

If multiple callbacks complete simultaneously, each is sent as a separate `AgentRequest::CallbackResult`. The agent worker processes them sequentially, one turn per callback. Each turn produces a visible response in the TUI. Callbacks are processed in completion order (oldest `completed_at` first).

### TUI Not Running

When the TUI starts, the callback poll runs immediately (not just on interval). Any tasks with `status = 'completed' AND trigger_type = 'callback' AND action_type = 'resume_agent'` scoped to the current agent are picked up and injected. Bounded to tasks completed within the last 7 days to prevent processing dozens of stale callbacks after a long absence.

### Agent Switching

Callback polls are scoped by `agent_id`. When the user switches agents via `/agent`, the poll switches to the new agent's callbacks. Pending callbacks for the previous agent remain in DB and are picked up when switching back (treated as startup recovery).

### Multiple TUI Instances

The `completed → delivered` transition is atomic: `UPDATE tasks SET status = 'delivered' WHERE status = 'completed' AND id = ?`. Returns affected rows — if 0, another instance already claimed it. Only the claiming instance runs the agent turn.

### Callback Result Size

The existing 100KB limit from `mika ask --task-id` is preserved. The callback result is injected as the "user message" for the agent turn, wrapped in `<callback_result>` tags. The agent summarizes and responds — the user sees the agent's processed response, not the raw result.

### Failed Agent Turn on Callback

If `run_agent()` fails during a callback turn (API error, timeout), the task stays `completed` (not `delivered`). Next idle poll picks it up again. Track retry count in a transient field; after 3 failures, mark the task `failed` and show the raw result to the user as a system message with an error note.

### Team Mode

Callback detection is not active in team mode (same as cross-channel polling). Team mode uses a different tick handler (`tick_team_mode`). Callbacks for the agent accumulate and are processed when returning to normal chat mode.

## System-Wide Impact

- **Interaction graph:** `mika ask --task-id` → marks task completed → TUI tick detects → `AgentRequest::CallbackResult` → agent worker → `run_agent()` → conversation stored → TUI renders → task marked `delivered`.

- **Error propagation:** If `run_agent()` fails during a callback turn, the task stays `completed` (not `delivered`). Next tick picks it up again. After 3 failures, mark as `failed` to prevent infinite retry.

- **State lifecycle risks:** The `delivered` status is terminal — once marked, the callback is never re-processed. The `completed → delivered` transition is atomic (SQL `UPDATE ... WHERE status = 'completed'`).

- **API surface parity:** Server mode gets the same `delivered` transition (after `dispatch_resume_agent` succeeds). Both paths follow: `completed → delivered`.

- **Integration test scenarios:**
  1. Long-running skill completes while TUI is active → result appears in conversation
  2. Long-running skill completes while TUI is closed → next TUI startup shows result
  3. Callback turn attempts long-running tool → blocked, agent responds with text only
  4. Two callbacks arrive while agent processes user message → queued, processed sequentially after idle
  5. Task cancelled before callback → `mika ask --task-id` gets "cannot complete" error, nothing injected
  6. Agent switch with pending callback → callback waits, delivered on switch-back

## Acceptance Criteria

- [x] TUI shows callback results in the conversation when a long-running task completes
- [x] TUI picks up undelivered callbacks from previous sessions on startup (bounded to 7 days)
- [ ] Footer task count includes pending callback tasks (not just reminders)
- [x] Callback turns cannot spawn new long-running tasks (code + prompt guard)
- [x] `mika ask --task-id` only marks task complete — no silent agent run
- [x] Tasks transition to `delivered` after successful TUI display
- [x] Server mode also marks tasks `delivered` after successful dispatch
- [x] Callback results stored as `role = 'tool_result'` in conversations
- [ ] Poll interval is configurable (default 5s, minimum 1s, maximum 60s) — deferred (hardcoded 5s is sufficient)
- [x] Multiple TUI instances don't double-process callbacks (atomic claiming)
- [x] All existing tests pass (~909 tests)
- [ ] New tests cover: callback injection, delivery lifecycle, loop prevention, startup recovery, retry-on-failure

## Implementation Phases

### Phase 1: Schema & Types (Foundation)

**`crates/mika-agent/src/task_engine/types.rs`:**
- Add `pub const DELIVERED: &str = "delivered";` to `task_status` module

**`crates/mika-agent/src/db.rs`:**
- Bump `CURRENT_SCHEMA_VERSION` to `2`
- Add `migrate_v2()`: recreate `tasks` table with `'delivered'` in status CHECK constraint; recreate `conversations` table with `'tool_result'` in role CHECK constraint. Wrap in transaction for atomicity.
- Add `get_undelivered_callback_tasks(agent_id) -> Vec<Task>` — query: `WHERE status = 'completed' AND trigger_type = 'callback' AND action_type = 'resume_agent' AND agent_id = ? AND completed_at > ? ORDER BY completed_at ASC`
- Add `mark_task_delivered(task_id) -> Result<bool>` — atomic: `UPDATE tasks SET status = 'delivered', updated_at = unixepoch() WHERE id = ? AND status = 'completed'`, returns false if already claimed
- Update `prune_old_tasks()` to include `delivered` in terminal statuses
- Update any terminal-status checks (grep for `completed.*failed.*expired.*cancelled`) to include `delivered`

**`crates/mika-agent/src/async_db.rs`:**
- Add async wrappers: `get_undelivered_callback_tasks()`, `mark_task_delivered()`

**Success criteria:** Schema v2 migration works for both new and existing v1 databases. All existing tests pass.

### Phase 2: `mika ask --task-id` Simplification

**`crates/mika-cli/src/commands/ask.rs`:**
- Remove the silent agent run block (lines 84-105): no more `run_silent_agent()`, `ToolRegistry`, `SkillRegistry`, `AtomicBool` setup
- Keep: task validation (lines 47-72), `update_task_completed()` (lines 73-82), `try_complete_parent_on_sibling_done()` (lines 110-116)
- The function now: validates task → marks complete → checks siblings → exits

**Success criteria:** `mika ask --task-id` marks task complete and exits cleanly. Parent task dispatch still works via sibling completion check.

### Phase 3: TUI Callback Injection

**`crates/mika-cli/src/tui/app.rs`:**
- Add `AgentRequest::CallbackResult { task_id: String, label: String, result: String }` variant to the enum (line 54)
- Add `poll_callback_tasks()` method: guarded by `AgentStatus::Idle`, queries `get_undelivered_callback_tasks()`, sends `AgentRequest::CallbackResult` for each (oldest first), atomically claims via `mark_task_delivered()` before sending
- Call `poll_callback_tasks()` from `tick()` at the configured poll interval (alongside existing cross-channel and task-count polls at line 642)
- On TUI startup (in `run()` or `new()`), run an immediate callback poll (bounded to 7 days)

**`crates/mika-cli/src/commands/chat.rs`:**
- Handle `AgentRequest::CallbackResult` in the agent worker match (line 142+):
  - Save callback result to conversations as `role = 'tool_result'`, `channel_type = 'callback'`, with `metadata = {"callback_task_id": "...", "label": "..."}`
  - Construct framing message: `"A background task has completed.\n\nTask: '{label}'\n\n<callback_result trust=\"untrusted\">\n{result}\n</callback_result>"`
  - Run `agent::run_agent()` with `is_callback_turn: true`, using framing as user message, `channel_type: "cli"`
  - On success: task already claimed (mark_task_delivered was called before sending)
  - On failure: increment retry counter; if < 3, the task stays `completed` for next poll (need to un-claim or track retries separately); if >= 3, mark `failed` and push raw result as system message to TUI

**Success criteria:** Callback results appear in TUI conversation. Agent processes results and responds naturally.

### Phase 4: Loop Prevention & Agent Params

**`crates/mika-agent/src/agent.rs`:**
- Add `is_callback_turn: bool` field to `AgentParams` (line 536). Default `false` in all existing call sites.
- In `run_agent()` / `run_agent_inner()`: when `is_callback_turn`, pass `long_running_ctx = None` instead of `Some(&lr_ctx)` (around line 786)

**`crates/mika-agent/src/prompt.rs`:**
- Add `callback_context: Option<&'a str>` to `PromptContext` (line 94)
- In `build_system_prompt()`: when `callback_context.is_some()`, inject section after instructions (~line 215):
  ```
  ## Callback Result Turn
  IMPORTANT: This is a callback result turn. A background task has completed and the results
  are provided below. You MUST NOT submit new long-running tasks. Process the results and
  respond directly to the user with your analysis.
  ```

**Success criteria:** Callback turns structurally cannot spawn long-running tasks. System prompt instructs agent to process results directly.

### Phase 5: Footer & Server Alignment

**`crates/mika-agent/src/db.rs`:**
- Update `get_pending_reminder_tasks()` (line 955) to broaden the query: count all pending/in_progress tasks except `action_type = 'run_skill'` (excludes heartbeat/reflection but includes callbacks and reminders)

**`crates/mika-agent/src/server/handlers.rs`:**
- After successful `dispatch_resume_agent` in `handle_task_complete` (around line 454 `Ok(()) => {}`): call `mark_task_delivered(task_id)`

**`crates/mika-agent/src/task_engine/dispatcher.rs`:**
- After successful `run_silent_agent` in `dispatch_resume_agent` (line 228+): call `db.mark_task_delivered(task_id)` via the dispatcher's db reference

**Success criteria:** Footer shows callback tasks in the count. Server mode marks tasks `delivered` after dispatch. Consistent lifecycle across CLI and server.

### Phase 6: Configuration & Polish

**`config/default.toml`:**
- Add `callback_poll_interval_secs = 5`

**`crates/mika-common/src/config.rs`:**
- Add `callback_poll_interval_secs: u64` field to Settings with `#[serde(default = "default_callback_poll_interval")]` (default 5)
- Validation: clamp to 1..=60

**`crates/mika-cli/src/tui/app.rs`:**
- Use `settings.callback_poll_interval_secs` to compute poll interval ticks instead of hardcoded constant
- Render `tool_result` messages distinctly in the conversation view — show with a `[{label} result]` header line, styled differently (e.g., dimmed or colored)

**`crates/mika-cli/src/tui/ui.rs`:**
- Handle `role = "tool_result"` in message rendering: show label from metadata, use distinct styling

**Success criteria:** Poll interval configurable via settings. Callback results rendered with visual distinction in TUI.

## Success Metrics

- Long-running task results are visible in the TUI within `poll_interval` seconds of completion
- No infinite callback → task loops (verified by test)
- Server mode continues to work as before with added `delivered` lifecycle
- Zero regression in existing test suite

## Dependencies & Risks

**Dependencies:**
- Schema v2 migration must be backward-compatible (v1 → v2 upgrade path)
- `mika ask --task-id` behavior change: scripts relying on silent agent output from `mika ask --task-id` will see changed behavior (no agent run, just mark-and-exit)

**Risks:**
- **SQLite table recreation** in v2 migration: data loss if interrupted mid-migration. Mitigate with transaction wrapping.
- **Retry logic complexity** in Phase 3: claiming a task then needing to un-claim on agent failure adds state management. Consider a simpler approach: only claim (`mark_task_delivered`) AFTER the agent turn succeeds, not before. This means the idle guard prevents double-processing (only one TUI polls while idle), and the atomic claim is the final step.
- **Terminal status grep:** Missing a `delivered` check in any terminal-status query could cause subtle bugs (e.g., parent tasks never dispatching). Thorough grep required in Phase 1.

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-07-callback-tui-delivery-brainstorm.md](docs/brainstorms/2026-03-07-callback-tui-delivery-brainstorm.md) — Key decisions: inject into TUI loop (not silent agent), `tool_result` role, code+prompt loop guard, `delivered` status, configurable poll interval, server mode unchanged.

### Internal References

- Agent worker channel: `crates/mika-cli/src/tui/app.rs:54-64` (AgentRequest enum)
- TUI tick loop: `crates/mika-cli/src/tui/app.rs:569` (tick method)
- Cross-channel polling pattern: `crates/mika-cli/src/tui/app.rs:642` (poll_cross_channel_messages)
- AgentStatus::Idle guard: `crates/mika-cli/src/tui/app.rs` (used by cross-channel poller)
- Task status constants: `crates/mika-agent/src/task_engine/types.rs:1-35`
- Schema migration: `crates/mika-agent/src/db.rs:298` (migrate function)
- AgentParams: `crates/mika-agent/src/agent.rs:536-564`
- Long-running interception: `crates/mika-agent/src/agent.rs:786` (lr_ctx passed to run_loop)
- Silent agent pattern: `crates/mika-agent/src/agent.rs:1354` (None for long_running_ctx)
- Footer rendering: `crates/mika-cli/src/tui/ui.rs:527-533`
- Server handler: `crates/mika-agent/src/server/handlers.rs:281-459`
- Callback loop prevention: `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`
- Callback lifecycle: `docs/solutions/architecture/callback-resume-agent-lifecycle.md`
- Background agent checklist: `docs/solutions/code-review-patterns/background-agent-mode-design-checklist.md`

### Related Work

- Unified task engine brainstorm: `docs/brainstorms/2026-03-04-unified-task-engine-brainstorm.md`
- Branch: `feat/unified-task-engine` (merged to main)
