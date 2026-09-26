---
title: "Conversation-mode max_steps=10 too low for autonomous task dispatch"
category: runtime-errors
date: 2026-04-03
tags: [agent-core, max-steps, step-limit, conversation-mode]
issue: "#386"
---

# Conversation-mode max_steps=10 too low for autonomous task dispatch

## Problem

mika-dev hit the 10-step conversation-mode limit before completing the autonomous dispatch sequence (fetch issue -> research code -> create task -> launch claude-pilot). The agent got cut off during research at step 9 and never reached task creation.

## Root cause

`MAX_TOOL_STEPS` was set to 10 while `MAX_CALLBACK_TOOL_STEPS` and `MAX_TEAM_TOOL_STEPS` were already at 20. The conversation-mode budget was the outlier. Complex autonomous workflows like issue dispatch routinely need 10-12+ steps.

## Solution

Raised `MAX_TOOL_STEPS` from 10 to 20 in `crates/mika-agent/src/agent.rs`:

```rust
const MAX_TOOL_STEPS: usize = 20;
const MAX_CALLBACK_TOOL_STEPS: usize = 20;  // unchanged, separate constant
const MAX_TEAM_TOOL_STEPS: usize = 20;      // unchanged, separate constant
```

All three constants are now 20 but kept separate to allow independent adjustment if modes need to diverge again.

Side effects: `SilentTrigger::Heartbeat`, `Reflection`, and `SkillRun` also went from 10 to 20. This is acceptable — agents only use as many steps as they need, and the 5-minute total timeout (`AGENT_TOTAL_TIMEOUT_SECS = 300`) remains the hard safety ceiling.

## Prevention

- When raising step limits for one mode, audit all modes that share the constant. `MAX_TOOL_STEPS` is used by both conversation mode and silent non-callback triggers.
- The total timeout (300s) is the real safety net. With 20 steps at 30s max per tool, the theoretical worst case (600s) exceeds it, but typical tool calls complete in 1-5s.
- Step-awareness nudge at `max_steps - 2` fires for all modes. Verify the nudge timing makes sense when changing limits.

## Related

- #375 / PR #378 — raised callback max_steps from 10 to 20
- #397 / PR #404 — gave Reminder the 20-step callback budget
- `docs/solutions/runtime-errors/silent-callback-max-steps-exhaustion.md` — callback step exhaustion pattern
- `docs/solutions/runtime-errors/reminder-trigger-max-steps-exhaustion.md` — reminder step budget mismatch
