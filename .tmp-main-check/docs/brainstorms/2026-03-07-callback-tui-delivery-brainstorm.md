# Callback TUI Delivery — Brainstorm

**Date:** 2026-03-07
**Status:** Design complete, ready for planning

---

## What We're Building

Closing the gap between the expected and actual user experience for long-running task callbacks in CLI/TUI mode. The unified task engine (feat/unified-task-engine) correctly creates callback tasks, spawns background processes, and stores results — but the TUI never shows the results to the user.

**Expected UX (from the unified-task-engine brainstorm):**

1. User asks agent to analyze something
2. Agent calls long_running skill → gets "task submitted" → tells user "analysis running"
3. TUI shows the task as pending in footer
4. Minutes later, background completes → callback arrives
5. TUI updates: shows the results in the conversation
6. Agent processes results and responds normally

**What actually happens:**

- Steps 1-2: work correctly
- Step 3: TUI footer only counts reminders, not callback tasks
- Step 4: callback arrives, result stored in DB (row 73: `channel_type='outbound'`)
- Step 5: TUI never updates — user still sees "analysis running" as last message
- Step 6: silent agent runs with no message sender, can't deliver, gives up (row 74: "no outbound messaging channel configured")

---

## Why This Approach

**Inject into TUI conversation loop, not silent agent.** The current implementation runs a silent agent when a callback arrives in CLI mode. Silent agents have no connection to the TUI — they can't display messages, can't continue the conversation, and the user never sees results. The fix injects the callback result directly into the TUI's agent worker channel, triggering a normal conversation turn where the agent processes the result and responds visibly.

**Minimal changes to working code.** Server mode (Telegram) already works via silent agent + GatewayMessageSender. We leave that path untouched and only fix the CLI/TUI path.

---

## Key Decisions

### 1. Callback Delivery Mechanism: Inject into TUI Loop

**Decision:** When a callback task completes in CLI/TUI mode, the TUI tick loop detects it and sends an `AgentRequest::CallbackResult` through the existing agent worker channel. The agent runs a normal conversation turn (not silent) with the callback result injected as context.

**Flow:**
1. `mika ask --task-id` marks the task as completed in DB (stores result) — no silent agent run
2. TUI tick loop polls for newly-completed callback tasks (status changed to `completed`, `action_type = 'resume_agent'`)
3. TUI injects `AgentRequest::CallbackResult { task_id, label, result }` into the agent worker channel
4. Agent worker runs a normal `run_agent()` turn with the callback result as the user message / injected context
5. Response appears in TUI conversation naturally

**Rejected alternative:** CliMessageSender bridge — would make results appear as disconnected notifications rather than natural conversation continuations.

### 2. Conversation Role: `tool_result` (New Role)

**Decision:** Add `tool_result` to the conversations role CHECK constraint. Callback results stored with `role = 'tool_result'` and metadata containing `callback_task_id` and `skill` name.

**Rationale:** Semantically accurate — it IS a tool result that came back asynchronously. Distinguishes it from user input, assistant responses, and system messages. Allows the TUI to render it distinctly (e.g., `[analyze_codebase result]`).

**Schema change:**
```sql
role CHECK (role IN ('user','assistant','system','summary','tool_result'))
```

### 3. Loop Prevention: Code + Prompt Guard

**Decision:** Enforce at both levels that a callback result turn cannot spawn new long-running tasks.

**Code guard:** During callback turns, override `long_running = false` on all skills before executing the agent loop. The agent literally cannot create a new callback task.

**Prompt guard:** Inject a system prompt instruction:
```
IMPORTANT: This is a callback result turn. You MUST NOT submit new long-running tasks. Process the results and respond directly.
```

Defense in depth — the code guard is the enforcement, the prompt guide prevents the agent from even attempting.

### 4. TUI Footer: Unified Task Count

**Decision:** Expand the footer task count query to include callback tasks, not just reminders. Show all pending/in_progress tasks excluding heartbeat/reflection (`action_type != 'run_skill'`).

```sql
SELECT COUNT(*) FROM tasks
WHERE agent_id = ? AND status IN ('pending', 'in_progress')
  AND action_type != 'run_skill'
```

### 5. Task Lifecycle: `delivered` Status

**Decision:** Add a `delivered` terminal status to the task lifecycle:

```
pending → in_progress → completed → delivered
```

`completed` means the result is stored but not yet shown to the user. The delivery layer (TUI or server) picks it up, delivers it, then marks it `delivered`.

**TUI:** Polls for `status = 'completed' AND action_type = 'resume_agent'`. After injecting into conversation and agent responds successfully: mark `delivered`.

**Server:** After successful `GatewayMessageSender` delivery in `dispatch_resume_agent`, mark `delivered`. Both CLI and server follow the same lifecycle.

**This cleanly handles:**
- TUI distinguishing "already shown" vs "needs to be shown"
- TUI not running when callback arrives — next TUI start picks up all `completed` (not yet `delivered`) callbacks
- Consistent lifecycle across all channels
- Idempotent delivery — a task can only be delivered once

**Schema change:**
```sql
status CHECK (status IN ('pending','in_progress','completed','failed','cancelled','expired','recurring_active','delivered'))
```

### 6. Callback Poll Frequency: Configurable

**Decision:** TUI polls for completed callbacks every N seconds (default: 5). Configurable via settings with a minimum of 1 second. The query is a local SQLite read — negligible cost.

On TUI startup, also check for any undelivered completed callbacks from previous sessions and inject them immediately.

### 7. Server Mode: Unchanged (+ delivered status)

**Decision:** Keep the server mode callback path as-is (silent agent + GatewayMessageSender → Telegram). It works. Only fix the CLI/TUI path. Add `delivered` status transition after successful delivery in both paths.

### 8. `mika ask --task-id` Behavior Change

**Decision:** In callback mode, `mika ask --task-id` should ONLY mark the task as completed and store the result. It should NOT run a silent agent. The TUI tick loop (or server dispatcher) handles the agent turn.

**Current behavior:** marks task complete → runs silent agent (which fails in CLI because no message sender)
**New behavior:** marks task complete → exits. The TUI detects the completion and handles delivery.

**Edge case:** If no TUI is running (e.g., `mika ask --task-id` called while TUI is closed), the result is stored in DB. Next time the TUI starts, it can detect unprocessed completed callbacks and inject them.

---

## Scope of Changes

### Files to modify:

1. **`crates/mika-cli/src/commands/ask.rs`** — Remove silent agent run for `--task-id` mode. Just mark task complete and exit.

2. **`crates/mika-cli/src/commands/chat.rs`** — Add callback polling to the TUI tick loop. When a completed callback is detected, send `AgentRequest::CallbackResult` to the agent worker.

3. **`crates/mika-cli/src/commands/chat.rs` (agent worker)** — Handle `AgentRequest::CallbackResult` variant. Run a normal agent turn with callback context injected. Apply long_running=false guard.

4. **`crates/mika-agent/src/db.rs`** — Add schema migration for `tool_result` role and `delivered` status. Add query for detecting newly-completed callback tasks (`status = 'completed' AND action_type = 'resume_agent'`). Add `update_task_delivered()` method.

5. **`crates/mika-agent/src/prompt.rs`** — Add callback turn prompt guard ("do not spawn long-running tasks").

6. **`crates/mika-cli/src/tui/app.rs`** — Update footer task count query to include callbacks (not just reminders).

7. **`crates/mika-agent/src/agent.rs`** — Add `is_callback_turn` flag to agent params. When set, override `long_running = false` on all skills.

8. **`crates/mika-agent/src/server/handlers.rs`** — After successful `dispatch_resume_agent`, mark task as `delivered`.

9. **`crates/mika-agent/src/task_engine/types.rs`** — Add `DELIVERED` status constant.

### Files NOT modified:

- `crates/mika-agent/src/task_engine/dispatcher.rs` — Server-side dispatch stays as-is
- `crates/mika-agent/src/skills/executor.rs` — Long-running task creation stays as-is

---

## Open Questions

*None — all questions resolved during brainstorm.*
