---
title: "fix: mika-dev callback phase hits max_steps=10 before acting on QA verdict"
type: fix
status: completed
date: 2026-04-01
issue: "#375"
---

# fix: Callback phase hits max_steps=10 before acting on QA verdict

## Overview

Silent-mode callback turns share the same `MAX_TOOL_STEPS = 10` limit as lightweight heartbeats and reflections. Complex callbacks (e.g., claude-pilot → mika-dev → mika-qa delegation chain) routinely consume all 10 steps on discovery and delegation, leaving zero steps for acting on the result. Worse, when silent mode exceeds max steps, the `LoopResult` is silently discarded — no continuation turn, no nudge, no notification.

## Problem Statement

From `/mika-dev-run-audit` of task b34d2725:
- mika-dev hit `agent exceeded max tool steps` at 18:43:43
- QA returned BLOCK verdict (Docs Sync CI failure)
- mika-dev started investigating but was forcibly terminated
- Work item left `in_progress` with no status update

**Root causes (three independent deficiencies):**

1. **Insufficient step budget**: `LoopMode::Silent` gets 10 steps — same as a heartbeat check-in — despite callbacks needing 12-15+ steps for PR discovery, QA delegation, verdict processing, and status updates.
2. **No step-awareness nudge**: Silent mode is excluded from the nudge at `max_steps - 2` (only Conversation and Team modes get it). The agent has no warning before hitting the wall.
3. **No graceful degradation**: `run_silent_inner()` calls `run_loop(...).await?;` and discards the `LoopResult`. No continuation turn. No fallback message. No user notification. The callback is silently swallowed.

## Proposed Solution

Layered defense — all three fixes implemented together:

### Fix 1: Per-trigger step limits via `LoopMode::Silent(usize)`

Change `LoopMode::Silent` from a unit variant to carry a `max_steps` field. Each `SilentTrigger` type specifies its own limit:

| Trigger | Current | New | Rationale |
|---------|---------|-----|-----------|
| Callback | 10 | 20 | Complex multi-delegation workflows |
| Heartbeat | 10 | 10 | Lightweight health checks |
| Reflection | 10 | 10 | Memory consolidation |
| SkillRun | 10 | 10 | Bounded by skill scope |
| Reminder | 10 | 10 | Typically single-action |

**File: `crates/mika-agent/src/agent.rs`**

```rust
// Constants
const MAX_TOOL_STEPS: usize = 10;
const MAX_CALLBACK_TOOL_STEPS: usize = 20;
const MAX_TEAM_TOOL_STEPS: usize = 20;

// LoopMode change
enum LoopMode {
    Conversation,
    Team,
    Silent { max_steps: usize },  // was: Silent
}

impl LoopMode {
    fn max_steps(&self) -> usize {
        match self {
            Self::Team => MAX_TEAM_TOOL_STEPS,
            Self::Silent { max_steps } => *max_steps,
            _ => MAX_TOOL_STEPS,
        }
    }
}
```

Each `SilentTrigger` variant provides its step limit:

```rust
impl SilentTrigger {
    fn max_steps(&self) -> usize {
        match self {
            Self::Callback { .. } => MAX_CALLBACK_TOOL_STEPS,
            _ => MAX_TOOL_STEPS,
        }
    }
}
```

Call site in `run_silent_inner()`:
```rust
let loop_mode = LoopMode::Silent { max_steps: trigger.max_steps() };
```

### Fix 2: Step-awareness nudge for silent mode

Extend the nudge condition at line ~512 to include `LoopMode::Silent`:

```rust
if step == max_steps - 2
    && let Some(ref mut system) = request.system
{
    let nudge = match mode {
        LoopMode::Conversation | LoopMode::Team => {
            "[SYSTEM: You have 2 tool steps remaining before the limit. \
             Prioritize completing your current task or summarizing progress.]"
        }
        LoopMode::Silent { .. } => {
            "[SYSTEM: You have 2 tool steps remaining before the limit. \
             Prioritize completing your current action or notifying the user via send_message.]"
        }
    };
    system.push_str(&format!("\n\n{nudge}"));
}
```

### Fix 3: Continuation turn for silent mode

After `run_loop()` returns in `run_silent_inner()`, inspect `LoopResult.max_steps_exceeded`. If true:

1. Make one more LLM call with tools disabled (60s timeout), asking for a summary
2. Save the summary to the session DB (already the case since `saves_to_db()` is true for Silent)
3. Send via `message_sender` if available — the user should know work was incomplete
4. Log a `warn!` with the trigger type and step count

```rust
let result = run_loop(/* ... */).await?;

if result.max_steps_exceeded {
    warn!(
        trigger = %trigger_name,
        max_steps,
        "silent agent exceeded max tool steps"
    );

    // Continuation turn — strip tools, ask for summary
    let summary = attempt_continuation_turn(&mut messages, &request, &provider, 60).await;

    if let Some(ref sender) = message_sender {
        let msg = summary.unwrap_or_else(|| format_step_exceeded_fallback(&messages));
        let _ = sender.send(&format!(
            "[Background task ran out of tool steps]\n\n{msg}"
        )).await;
    }
}
```

Extract the continuation turn logic from `run_agent()` into a shared helper (`attempt_continuation_turn`) reusable by both Conversation and Silent modes.

## Technical Considerations

### Total timeout interaction

`AGENT_TOTAL_TIMEOUT_SECS = 300` wraps the entire silent agent run. With 20 steps and `delegate_task` at 120s timeout, the total timeout may become binding. However:
- Not all 20 steps involve delegations — most are lightweight tool calls (search_memory, update_work_item_status, send_message)
- The 300s timeout is a safety net, not a budget. Most callbacks complete in 60-120s even with 15+ steps
- Raising the timeout creates a different risk: a stuck callback blocking the agent for 10+ minutes

**Decision:** Keep `AGENT_TOTAL_TIMEOUT_SECS = 300` unchanged. If timeout becomes the binding constraint in practice, address it as a separate issue.

### `is_callback_turn: false` in silent path

`run_silent_inner` sets `is_callback_turn: false` on `ToolContext`. This looks like a gap, but `long_running: None` at line 1915 separately prevents long-running task creation. The two guards are independent:
- `is_callback_turn` blocks `create_work_item` (tool-level guard)
- `long_running: None` blocks long-running task spawning (context-level guard)

**Decision:** Leave `is_callback_turn: false` as-is. Add a code comment explaining the guard layering. The silent callback path uses `is_task_context: true` for its own loop prevention.

### Cost implications

Raising callback max_steps from 10 to 20 doubles the worst-case LLM API calls per callback. However:
- Most callbacks will still complete in 12-15 steps (early exit on EndTurn)
- Callbacks are infrequent (triggered by external process completion, not periodic)
- The alternative (silently dropping work) is far more costly in operational terms

## System-Wide Impact

- **`LoopMode` enum change**: All `match` arms on `LoopMode::Silent` must be updated to destructure the new field. Grep for `LoopMode::Silent` — expect ~8-12 match sites in agent.rs and tests.
- **Continuation turn extraction**: Refactoring the continuation turn into a shared helper touches `run_agent()` (Conversation), `run_team_agent()` (Team), and `run_silent_inner()` (Silent). The helper must be generic enough for all three callers.
- **No config/schema/API changes**: This is a pure engine-internal change. No new env vars, no DB schema migration, no HTTP API changes.

## Acceptance Criteria

- [x] `LoopMode::Silent` carries a `max_steps: usize` field
- [x] `SilentTrigger::Callback` gets 20 steps; other triggers keep 10
- [x] Step-awareness nudge fires at `max_steps - 2` for all `LoopMode` variants
- [x] Silent nudge text is tailored ("notify the user via send_message")
- [x] `run_silent_inner()` handles `max_steps_exceeded`: continuation turn + send_message fallback
- [x] Continuation turn logic extracted into a shared helper used by Conversation, Team, and Silent modes
- [x] `warn!` log emitted when silent mode exceeds max steps (includes trigger type)
- [x] All existing tests pass (`cargo test`)
- [x] New test: silent callback exceeding max steps triggers continuation turn
- [x] New test: `SilentTrigger::max_steps()` returns correct values per variant
- [x] Code comment on `is_callback_turn: false` in silent path explaining guard layering

## MVP

### crates/mika-agent/src/agent.rs

**1. Constants and LoopMode change:**

```rust
const MAX_TOOL_STEPS: usize = 10;
const MAX_CALLBACK_TOOL_STEPS: usize = 20;
const MAX_TEAM_TOOL_STEPS: usize = 20;

enum LoopMode {
    Conversation,
    Team,
    Silent { max_steps: usize },
}

impl LoopMode {
    fn max_steps(&self) -> usize {
        match self {
            Self::Team => MAX_TEAM_TOOL_STEPS,
            Self::Silent { max_steps } => *max_steps,
            _ => MAX_TOOL_STEPS,
        }
    }

    fn saves_to_db(&self) -> bool {
        !matches!(self, Self::Team)
    }
}
```

**2. SilentTrigger max_steps method:**

```rust
impl SilentTrigger {
    fn max_steps(&self) -> usize {
        match self {
            Self::Callback { .. } => MAX_CALLBACK_TOOL_STEPS,
            _ => MAX_TOOL_STEPS,
        }
    }
}
```

**3. Step nudge for all modes:**

```rust
// In run_loop(), replace the existing nudge condition:
if step == max_steps - 2
    && let Some(ref mut system) = request.system
{
    let nudge = match mode {
        LoopMode::Silent { .. } => {
            "[SYSTEM: You have 2 tool steps remaining before the limit. \
             Prioritize completing your current action or notifying the user via send_message.]"
        }
        _ => {
            "[SYSTEM: You have 2 tool steps remaining before the limit. \
             Prioritize completing your current task or summarizing progress.]"
        }
    };
    system.push_str(&format!("\n\n{nudge}"));
}
```

**4. Shared continuation turn helper:**

```rust
async fn attempt_continuation_turn(
    messages: &[LlmMessage],
    system: &str,
    provider: &dyn LlmProvider,
    model: &str,
    timeout_secs: u64,
) -> Option<String> {
    // Build request with no tools, append "summarize what you accomplished" user message
    // Call provider with timeout
    // Return the text response or None on failure
}
```

**5. Silent mode max_steps handling in run_silent_inner():**

```rust
let result = run_loop(/* ... */).await?;

if result.max_steps_exceeded {
    warn!(
        trigger = %trigger_name,
        max_steps = trigger.max_steps(),
        "silent agent exceeded max tool steps"
    );

    let summary = attempt_continuation_turn(/* ... */).await;

    if let Some(ref sender) = message_sender {
        let text = summary.unwrap_or_else(|| {
            format_step_exceeded_fallback(&messages)
        });
        let _ = sender.send(&format!(
            "[Background task exceeded tool step limit]\n\n{text}"
        )).await;
    }
}
```

### Tests

**crates/mika-agent/src/agent.rs (inline tests):**

```rust
#[test]
fn test_silent_trigger_max_steps() {
    // Callback gets 20
    let trigger = SilentTrigger::Callback { /* fields */ };
    assert_eq!(trigger.max_steps(), 20);

    // Others get 10
    let trigger = SilentTrigger::Heartbeat;
    assert_eq!(trigger.max_steps(), 10);
}

#[test]
fn test_loop_mode_silent_max_steps() {
    assert_eq!(LoopMode::Silent { max_steps: 20 }.max_steps(), 20);
    assert_eq!(LoopMode::Silent { max_steps: 10 }.max_steps(), 10);
}
```

## Sources

- Related issue: #375
- Solution doc: [agent-max-steps-no-followup.md](docs/solutions/runtime-errors/agent-max-steps-no-followup.md) — continuation turn pattern
- Solution doc: [team-agent-max-steps-exhaustion-no-output.md](docs/solutions/runtime-errors/team-agent-max-steps-exhaustion-no-output.md) — team mode continuation turn
- Solution doc: [callback-task-loop-prevention.md](docs/solutions/architecture-patterns/callback-task-loop-prevention.md) — guard layering
- Key file: `crates/mika-agent/src/agent.rs` — constants (L31-34), `LoopMode` (L200), `run_loop` (L497-758), nudge (L511-520), `run_agent` continuation (L1214-1283), `run_silent_inner` (L1670+)
- Key file: `crates/mika-agent/src/task_engine/dispatcher.rs` — `dispatch_resume_agent` (L278)
