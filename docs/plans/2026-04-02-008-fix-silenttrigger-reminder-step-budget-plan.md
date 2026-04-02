---
title: "fix: SilentTrigger::Reminder gets 10-step budget, should match Callback's 20"
type: fix
status: completed
date: 2026-04-02
---

# fix: SilentTrigger::Reminder gets 10-step budget, should match Callback's 20

## Overview

`SilentTrigger::max_steps()` uses a wildcard `_ => MAX_TOOL_STEPS` (10) catch-all that gives Reminder-triggered agent runs only 10 tool steps. Reminders created via `create_reminder` with `action_type: resume_agent` are the continuation mechanism for callback workflows (e.g., CI follow-up after a dev run). They need the same 20-step budget as callbacks.

## Problem Statement

During the mika#381 dev run (task `654ff1dc`), mika-dev's callback turn set a CI follow-up reminder. When the reminder fired as `SilentTrigger::Reminder`, it got only 10 steps. The agent needed to: check CI, delegate to mika-qa, parse verdict, update work item, notify user, AND launch a retry session — but hit `agent exceeded max tool steps` after exhausting its budget on 6 necessary actions and 4 exploratory tool calls.

## Proposed Solution

Add `Self::Reminder { .. }` to the `Callback` match arm in `SilentTrigger::max_steps()`, and make the match exhaustive (no wildcard) so the compiler forces a decision when new variants are added.

## Acceptance Criteria

- [x] `SilentTrigger::Reminder` returns `MAX_CALLBACK_TOOL_STEPS` (20) from `max_steps()`
- [x] Match is exhaustive — no wildcard `_` arm — so adding a new variant is a compile error
- [x] Test `test_silent_trigger_non_callback_gets_default_step_limit` updated: Reminder assertion removed
- [x] Existing test confirms Reminder gets `MAX_CALLBACK_TOOL_STEPS`
- [x] `cargo test -p mika-agent` passes
- [x] `cargo clippy` clean

## Implementation

### `crates/mika-agent/src/agent.rs`

**1. Update `SilentTrigger::max_steps()` (~line 1649):**

```rust
fn max_steps(&self) -> usize {
    match self {
        Self::Callback { .. } | Self::Reminder { .. } => MAX_CALLBACK_TOOL_STEPS,
        Self::Heartbeat | Self::Reflection | Self::SkillRun { .. } => MAX_TOOL_STEPS,
    }
}
```

Key change: explicit match arms (no wildcard) + Reminder joined with Callback.

**2. Update `test_silent_trigger_callback_gets_higher_step_limit` (~line 2588):**

Add Reminder assertion alongside Callback:

```rust
#[test]
fn test_silent_trigger_callback_gets_higher_step_limit() {
    let trigger = SilentTrigger::Callback {
        task_id: "test-task".to_string(),
        label: "test".to_string(),
        result: "done".to_string(),
        failed: false,
        parent_task_id: None,
    };
    assert_eq!(trigger.max_steps(), MAX_CALLBACK_TOOL_STEPS);

    let reminder = SilentTrigger::Reminder {
        task_id: "test".to_string(),
        message: "test".to_string(),
    };
    assert_eq!(reminder.max_steps(), MAX_CALLBACK_TOOL_STEPS);
}
```

**3. Update `test_silent_trigger_non_callback_gets_default_step_limit` (~line 2600):**

Remove the Reminder assertion (it no longer gets `MAX_TOOL_STEPS`):

```rust
#[test]
fn test_silent_trigger_non_callback_gets_default_step_limit() {
    assert_eq!(SilentTrigger::Heartbeat.max_steps(), MAX_TOOL_STEPS);
    assert_eq!(SilentTrigger::Reflection.max_steps(), MAX_TOOL_STEPS);
    assert_eq!(
        SilentTrigger::SkillRun {
            skill_name: "test".to_string()
        }
        .max_steps(),
        MAX_TOOL_STEPS
    );
}
```

## Sources

- GitHub issue: #397
- Prior learning: `docs/solutions/runtime-errors/silent-callback-max-steps-exhaustion.md` — predicted this exact scenario
- Architecture doc: `docs/solutions/architecture/reminder-resume-agent-dual-lifecycle.md` — #363 Reminder addition
