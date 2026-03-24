---
title: "fix: callback processing race steals TUI notifications and skips mika-qa review"
type: fix
status: completed
date: 2026-03-24
issue: 264
---

# fix: callback processing race steals TUI notifications and skips mika-qa review

## Overview

Two related bugs in the callback system prevent the self-dev workflow from completing properly. The task engine's `dispatch_undelivered_callbacks()` races with the TUI's `poll_callback_tasks()` and can steal claude-pilot callbacks, processing them in a silent turn that the user never sees. When the engine dispatcher handles a callback, it runs a generic "analyze and notify" instruction in a new session with no conversation history, causing the agent to skip mika-qa delegation and acceptance testing.

## Problem Statement

### Bug 1: TUI callback race

In CLI mode, two independent systems compete for callback tasks:

| System | Poll interval | Query scope | Claims when |
|--------|--------------|-------------|-------------|
| TUI `poll_callback_tasks()` | ~5s (167 ticks x 30ms) | Session-scoped | Before processing (atomic `mark_task_delivered`) |
| Engine `dispatch_undelivered_callbacks()` | 60s (DB_SCAN_INTERVAL_TICKS) | All agent callbacks | After silent turn completes |

When the engine wins the race:
1. Engine runs `dispatch_resume_agent` -> `run_silent_agent(SilentTrigger::Callback)` in a new `callback-{uuid}` session (no history)
2. Engine marks the task as `delivered` after the turn
3. TUI's next poll finds nothing (already delivered)

The TUI path is superior: it runs `run_agent()` within the existing conversation session with full history and `is_callback_turn: true`.

### Bug 2: Generic callback trigger context

When the engine dispatcher processes a callback via `run_silent_agent()`, the trigger context (`agent.rs:1450-1465`) says:

> "Analyze the data and use send_message to notify the user with a clear, concise summary."

The self-dev skill prompt IS injected (always_on), but:
- The callback creates a NEW `callback-{uuid}` session with no conversation history
- The agent doesn't know it was mid-workflow
- The generic instruction directs it to "analyze and notify" rather than "continue the workflow"
- Agent marks work item complete and stops, skipping Steps 5/5.5/6 (acceptance testing, mika-qa delegation)

## Proposed Solution

### Fix 1: Skip engine callback dispatch in CLI mode

Add a `cli_mode: bool` field to `TaskDispatcher`. Guard `dispatch_undelivered_callbacks()` behind `!cli_mode`. In CLI mode, the TUI's `poll_callback_tasks()` is the sole callback consumer.

**Invariant:** `agent_lock: None` in CLI mode is safe because the engine never dispatches callbacks (no concurrent silent agent runs to serialize).

### Fix 2: Workflow-aware callback trigger for self-dev

In the `SilentTrigger::Callback` arm of `run_silent_agent()`, detect claude-pilot callbacks by exact label match and inject workflow-specific continuation instructions.

**Label detection:** `label == "long_running:run_claude_pilot"` (exact match, not substring — the label format `long_running:{tool_name}` is stable, set by `executor.rs:527`).

**Branching logic:**
- **Success + claude-pilot:** Inject self-dev workflow continuation (delegate to mika-qa, manage work items, notify user)
- **Failure + claude-pilot:** Inject failure escalation (extract failure details, notify user, do NOT retry)
- **All other callbacks:** Keep existing generic "analyze and notify" behavior

**Key constraint:** Silent mode uses `safe_always_on_skills()` — no exec/http handlers. But `delegate_task`, `update_work_item_status`, `send_message`, `check_work_item` ARE available (builtin tools). So the agent can delegate to mika-qa and manage work items, but cannot run shell-based acceptance tests directly.

## Technical Considerations

### Server-mode double-dispatch safety

In server mode, the handler spawns `dispatch_resume_agent` on `POST /tasks/{id}/complete`, and the engine also scans periodically. The `agent_lock: Some(mutex)` serializes these — the second caller gets `AgentBusy` and defers. By the next 60s scan, the first dispatch has already marked `delivered`, so the query `status IN ('completed', 'failed')` won't match. Safe.

### CLI self-dev callback (after Fix 1)

After Fix 1, CLI self-dev callbacks go through the TUI's `run_agent()` path — NOT `run_silent_agent()`. Fix 2's workflow instructions don't apply. The agent has full conversation history plus always-on self-dev skill prompt, which provides sufficient context for workflow continuation. This is a better outcome: the TUI agent has richer context than any static instruction we could inject.

### Loop prevention preservation

All four loop-prevention layers remain intact:
1. **Code guard:** `is_callback_turn` nullifies `LongRunningContext` (TUI path); silent mode uses `safe_always_on_skills()` filtering out exec/http handlers (server path)
2. **Prompt guard:** Callback trigger includes "MUST NOT submit new long-running tasks"
3. **Trust boundary:** `<callback_result trust="untrusted">` wrapping in `format_callback_framing()`
4. **Size limit:** 10KB truncation via `CALLBACK_RESULT_MAX_BYTES`

### Team mode callbacks (after Fix 1)

Both TUI polling (`!is_team_mode()` guard) and engine dispatch (`cli_mode: true` guard) are disabled. Callbacks accumulate in DB. Processed when team mode exits and TUI resumes polling. Acceptable — team runs are the primary focus during team mode.

## System-Wide Impact

- **Interaction graph:** Fix 1 only removes the engine's callback dispatch path in CLI mode — no new interactions. Fix 2 adds label-based branching in `run_silent_agent()` — the callback's `SilentTrigger::Callback` variant already has `label` available, so no new data flow.
- **Error propagation:** If Fix 2's workflow-aware instruction causes agent failure, `dispatch_resume_agent` does NOT mark delivered, so the engine retries on the next 60s scan. Pre-existing behavior, no new retry concerns.
- **State lifecycle risks:** No new state transitions. `mark_task_delivered` remains the sole state change. The `cli_mode` guard prevents a code path from executing — it doesn't introduce new state.
- **API surface parity:** No API changes. Both CLI and server modes handle callbacks correctly via their respective paths.

## Acceptance Criteria

- [x] In CLI mode, `dispatch_undelivered_callbacks()` is not called by the task engine
- [x] In server mode, `dispatch_undelivered_callbacks()` still runs normally
- [x] Claude-pilot callback (server mode) triggers self-dev workflow continuation instructions
- [x] Failed claude-pilot callbacks produce escalation notifications, not retries
- [x] Non-claude-pilot callbacks retain existing generic "analyze and notify" behavior
- [x] All existing tests pass with the new `cli_mode` field (3 test helper sites updated)
- [x] `cargo clippy` clean
- [x] `cargo test` passes

## Implementation

### Phase 1: Add `cli_mode` to TaskDispatcher (Fix 1)

#### 1.1 `crates/mika-agent/src/task_engine/dispatcher.rs`

Add `pub cli_mode: bool` field to the `TaskDispatcher` struct (after `agent_lock`).

```rust
pub struct TaskDispatcher {
    // ... existing fields ...
    pub agent_lock: Option<Arc<tokio::sync::Mutex<()>>>,
    /// When true, the engine skips `dispatch_undelivered_callbacks()`.
    /// CLI/TUI mode handles callbacks via `poll_callback_tasks()` instead.
    pub cli_mode: bool,
}
```

Update `test_dispatcher()` helper (line ~755): add `cli_mode: false`.

#### 1.2 `crates/mika-agent/src/task_engine/engine.rs`

Guard callback dispatch in the `tick()` method's DB scan block (line ~201):

```rust
if !self.dispatcher.cli_mode {
    self.dispatch_undelivered_callbacks().await;
}
```

Update `test_dispatcher()` helper (line ~592): add `cli_mode: false`.

#### 1.3 `crates/mika-cli/src/commands/chat.rs`

Set `cli_mode: true` in the CLI dispatcher construction (line ~120):

```rust
let dispatcher = Arc::new(TaskDispatcher {
    // ... existing fields ...
    agent_lock: None,
    cli_mode: true,
});
```

#### 1.4 `crates/mika-agent/src/server/mod.rs`

Set `cli_mode: false` in the server dispatcher construction (line ~237):

```rust
let dispatcher = Arc::new(TaskDispatcher {
    // ... existing fields ...
    agent_lock: Some(agent_lock.clone()),
    cli_mode: false,
});
```

Update `test_task_engine()` helper (line ~633): add `cli_mode: false`.

### Phase 2: Workflow-aware callback trigger (Fix 2)

#### 2.1 `crates/mika-agent/src/agent.rs`

Add a constant for the claude-pilot callback label:

```rust
const CLAUDE_PILOT_CALLBACK_LABEL: &str = "long_running:run_claude_pilot";
```

Modify the `SilentTrigger::Callback` arm in `run_silent_agent()` (line ~1450) to branch on label:

```rust
SilentTrigger::Callback {
    task_id,
    label,
    result,
    failed,
} => {
    let base = format_callback_framing(label, task_id, result, *failed);
    if label == CLAUDE_PILOT_CALLBACK_LABEL && !failed {
        format!(
            "{base}\n\
             IMPORTANT: A successful result confirms only the specific action performed. \
             NEVER extrapolate to downstream states (PR status, CI health, deploy readiness) \
             that the result does not explicitly mention.\n\n\
             This is a completed claude-pilot development session. Your workflow:\n\
             1. Extract the PR URL and key findings from the callback result above.\n\
             2. Use send_message to notify the user with: PR URL, summary of changes, and any warnings.\n\
             3. MANDATORY: Use delegate_task to delegate acceptance testing to mika-qa. \
                Include the PR URL and change summary in the delegation message. \
                Wait for mika-qa's verdict before proceeding.\n\
             4. Based on mika-qa's verdict:\n\
                - If APPROVED: Use update_work_item_status to mark the work item as completed.\n\
                - If REJECTED: Use send_message to notify the user of the rejection reason. \
                  Do NOT mark the work item as completed.\n\
             5. Use send_message to give the user a final status update."
        )
    } else if label == CLAUDE_PILOT_CALLBACK_LABEL && *failed {
        format!(
            "{base}\n\
             IMPORTANT: This claude-pilot development session has FAILED.\n\n\
             Your workflow:\n\
             1. Extract the failure details from the callback result above.\n\
             2. Use send_message to notify the user with: failure reason, any partial progress, \
                and recommended next steps.\n\
             3. Do NOT retry the development session — the user must decide how to proceed.\n\
             4. Do NOT mark the work item as completed. Leave it in its current status \
                so the user can decide whether to retry or cancel."
        )
    } else {
        format!(
            "{base}\n\
             IMPORTANT: A successful result confirms only the specific action performed. \
             NEVER extrapolate to downstream states (PR status, CI health, deploy readiness) \
             that the result does not explicitly mention.\n\
             Analyze the data and use send_message to notify the user \
             with a clear, concise summary. Include the key findings and any recommended actions."
        )
    }
}
```

### Phase 3: Tests

#### 3.1 Unit test: `cli_mode` guards callback dispatch

In `crates/mika-agent/src/task_engine/engine.rs` tests, add a test that verifies `dispatch_undelivered_callbacks()` is skipped when `cli_mode: true`:

- Create a task engine with `cli_mode: true`
- Insert a completed callback task
- Run multiple tick cycles past `DB_SCAN_INTERVAL_TICKS`
- Verify the callback task is NOT marked as delivered

Add a companion test with `cli_mode: false` verifying the callback IS dispatched.

#### 3.2 Unit test: Workflow-aware trigger text

In `crates/mika-agent/src/agent.rs` tests, add tests verifying the trigger context text for:
- `label == "long_running:run_claude_pilot"` + `failed: false` → contains "delegate_task" and "mika-qa"
- `label == "long_running:run_claude_pilot"` + `failed: true` → contains "FAILED" and "Do NOT retry"
- `label == "long_running:some_other_tool"` → contains "Analyze the data" (generic)

## Dependencies & Risks

- **No schema changes:** Both fixes modify Rust structs and logic only. No DB migration needed.
- **No API changes:** No HTTP endpoint or tool interface changes.
- **Risk: Hardcoded label in agent.rs** — `CLAUDE_PILOT_CALLBACK_LABEL` couples the agent engine to a specific skill name. Acceptable for now; a manifest-based `callback_instructions` mechanism can be added later if more skills need custom callback behavior.
- **Risk: CLI self-dev callbacks rely on conversation history** — After Fix 1, CLI self-dev callbacks go through `run_agent()` with conversation history. If the agent fails to continue the workflow from history alone, a parallel fix in `chat.rs` would be needed. Low risk: the always-on self-dev skill prompt + full history provides rich context.

## Sources & References

- Issue: [#264](https://github.com/senara-solutions/mika/issues/264)
- `crates/mika-agent/src/task_engine/dispatcher.rs:33-47` — TaskDispatcher struct
- `crates/mika-agent/src/task_engine/engine.rs:188-221` — tick() with DB scan block
- `crates/mika-agent/src/task_engine/engine.rs:314-370` — dispatch_undelivered_callbacks()
- `crates/mika-agent/src/agent.rs:1294-1309` — SilentTrigger enum
- `crates/mika-agent/src/agent.rs:1450-1465` — Callback trigger context
- `crates/mika-agent/src/agent.rs:72-107` — format_callback_framing()
- `crates/mika-cli/src/commands/chat.rs:120-136` — CLI dispatcher construction
- `crates/mika-cli/src/commands/chat.rs:294-383` — AgentRequest::CallbackResult handling
- `crates/mika-cli/src/tui/app.rs:1414-1519` — poll_callback_tasks()
- `crates/mika-agent/src/server/mod.rs:237-249` — Server dispatcher construction
- `crates/mika-agent/src/skills/executor.rs:527` — Label format: `long_running:{tool_name}`
- `docs/solutions/architecture-patterns/callback-tui-delivery-polling.md` — TUI polling architecture
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` — Loop prevention layers
- `docs/solutions/architecture/callback-resume-agent-lifecycle.md` — Full callback lifecycle
- `docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md` — Status filter audit pattern
