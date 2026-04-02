---
title: "SilentTrigger::Reminder gets 10-step budget instead of 20"
category: runtime-errors
date: 2026-04-02
tags: [silent-mode, reminder, max-steps, step-budget, agent-loop]
related_issues: ["#397", "#375", "#363"]
---

# SilentTrigger::Reminder gets 10-step budget instead of 20

## Problem

When a callback turn creates a CI follow-up reminder via `create_reminder` with `action_type: resume_agent`, the reminder fires as `SilentTrigger::Reminder` which got `MAX_TOOL_STEPS = 10` — not the `MAX_CALLBACK_TOOL_STEPS = 20` that callbacks receive. This caused the agent to hit `agent exceeded max tool steps` during complex reminder-triggered workflows that needed 12-15+ steps (e.g., check CI, delegate QA, parse verdict, update work item, notify user, launch retry).

## Root Cause

`SilentTrigger::max_steps()` used a wildcard catch-all that gave all non-Callback triggers the default 10-step budget:

```rust
fn max_steps(&self) -> usize {
    match self {
        Self::Callback { .. } => MAX_CALLBACK_TOOL_STEPS, // 20
        _ => MAX_TOOL_STEPS, // 10 — Reminder falls here
    }
}
```

The `Reminder` variant was added in #363 after the `max_steps()` method was introduced in #375. The wildcard arm silently assigned the wrong budget.

## Solution

Added `Self::Reminder { .. }` to the Callback match arm and replaced the wildcard with exhaustive match arms so the compiler forces a decision when new variants are added:

```rust
fn max_steps(&self) -> usize {
    match self {
        Self::Callback { .. } | Self::Reminder { .. } => MAX_CALLBACK_TOOL_STEPS,
        Self::Heartbeat | Self::Reflection | Self::SkillRun { .. } => MAX_TOOL_STEPS,
    }
}
```

Updated tests: moved the Reminder assertion from the "non-callback gets default" test to the "callback gets higher" test.

## Prevention

- **Use exhaustive matches instead of wildcards** for budget/limit selectors. The compiler then forces explicit decisions for new variants.
- The #375 solution doc (`silent-callback-max-steps-exhaustion.md`) warned: "When adding new SilentTrigger variants, always add a case to `SilentTrigger::max_steps()`." The `Reminder` variant added in #363 missed this guidance.
- When adding enum variants, grep for all `match self` blocks on that enum to ensure every match site is updated — especially those using wildcards.

## Related

- `docs/solutions/runtime-errors/silent-callback-max-steps-exhaustion.md` — #375 introduced `SilentTrigger::max_steps()` with the wildcard
- `docs/solutions/architecture/reminder-resume-agent-dual-lifecycle.md` — #363 added the `Reminder` variant
