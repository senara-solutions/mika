---
title: Silent callback max_steps exhaustion silently swallows results
category: runtime-errors
date: 2026-04-01
severity: high
tags: [agent-loop, callback, max-steps, silent-mode, continuation-turn]
issue: "#375"
---

# Silent Callback Max Steps Exhaustion

## Problem

When the silent agent loop processed complex callbacks (e.g., claude-pilot → mika-dev → mika-qa
delegation chain), the callback phase consumed all 10 tool steps on PR discovery, QA delegation,
and CI investigation — leaving zero steps for acting on the QA verdict. The task was left
`in_progress` with no status update and no user notification.

Three independent deficiencies compounded:

1. **Insufficient step budget**: `LoopMode::Silent` shared the same `MAX_TOOL_STEPS = 10` with
   lightweight heartbeats and reflections, despite callbacks needing 12-15+ steps.
2. **No step-awareness nudge**: Silent mode was excluded from the nudge at `max_steps - 2`
   (only Conversation and Team modes received it).
3. **No graceful degradation**: `run_silent_inner()` called `run_loop().await?` and discarded the
   `LoopResult` entirely — no continuation turn, no fallback, no user notification. The callback
   was silently swallowed.

## Root Cause

`LoopMode::Silent` was a unit variant with a hardcoded 10-step limit. The step-awareness nudge
condition explicitly excluded silent mode (`matches!(mode, LoopMode::Conversation | LoopMode::Team)`).
The `run_silent_inner()` function did not inspect `LoopResult.max_steps_exceeded` — the result was
propagated via `?` for errors but otherwise discarded.

## Solution

Layered defense with three fixes:

### 1. Per-trigger step limits via `LoopMode::Silent { max_steps }`

Changed `LoopMode::Silent` from a unit variant to `Silent { max_steps: usize }`. Each
`SilentTrigger` type specifies its own limit via `SilentTrigger::max_steps()`:

- `Callback` → `MAX_CALLBACK_TOOL_STEPS` (20)
- `Heartbeat`, `Reflection`, `SkillRun`, `Reminder` → `MAX_TOOL_STEPS` (10)

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

### 2. Step-awareness nudge for all modes

Extended the nudge condition to fire for all `LoopMode` variants at `max_steps - 2`. Silent mode
gets tailored text encouraging `send_message` notification:

```rust
if step == max_steps - 2 && let Some(ref mut system) = request.system {
    let nudge = match mode {
        LoopMode::Silent { .. } => "[SYSTEM: You have 2 tool steps remaining ...]",
        _ => "[SYSTEM: You have 2 tool steps remaining ...]",
    };
    // ...
}
```

### 3. Shared continuation turn helper

Extracted `attempt_continuation_turn()` from duplicate inline code in Conversation and Team modes.
Now used by all three modes. In silent mode, the summary is sent via `message_sender` with a
`[Background task exceeded tool step limit]` prefix.

```rust
if result.max_steps_exceeded {
    let cont = attempt_continuation_turn(&mut request, llm, &result, trigger_label).await;
    if let Some(ref sender) = params.message_sender {
        let _ = sender.send(&format!("[Background task exceeded tool step limit]\n\n{}", cont.text)).await;
    }
}
```

## Key Design Decisions

1. **`LoopMode::Silent { max_steps }` vs separate variants**: Embedding `max_steps` in the variant
   keeps `LoopMode` about behavioral mode while `SilentTrigger` owns per-trigger policy. Adding
   `SilentCallback`, `SilentHeartbeat`, etc. would duplicate all Silent match arms.

2. **Keep `AGENT_TOTAL_TIMEOUT_SECS = 300` unchanged**: With 20 steps, realistic callbacks complete
   in 80-160s. The 300s timeout is a safety net. If it becomes binding in practice, address separately.

3. **Leave `is_callback_turn: false` in silent path**: Loop prevention in silent mode relies on
   `long_running: None` (blocks task spawning) and `is_task_context: true` (blocks top-level work
   item creation). Added a code comment documenting this guard layering.

## Prevention

- When adding new `SilentTrigger` variants, always add a case to `SilentTrigger::max_steps()`.
  The default arm returns `MAX_TOOL_STEPS` (10), which is safe but may be insufficient.
- When modifying the `run_loop()` step nudge or continuation turn logic, verify all three modes
  (Conversation, Team, Silent) are covered — they share the same `attempt_continuation_turn()` helper.
- The continuation turn happens inside the 300s `AGENT_TOTAL_TIMEOUT_SECS` wrapper. If step counts
  are raised further, verify the total timeout still provides adequate headroom.

## Related

- [Agent Max-Steps Fallback Never Follows Up](agent-max-steps-no-followup.md) — original
  continuation turn pattern for Conversation mode
- [Team Agent Max-Steps Exhaustion](team-agent-max-steps-exhaustion-no-output.md) — same pattern
  applied to Team mode
- [Callback Result Too Large Causes Agent Timeout](callback-result-too-large-causes-agent-timeout.md)
  — related callback processing issue (10KB truncation cap)
- [Callback Task Loop Prevention](../architecture-patterns/callback-task-loop-prevention.md) —
  guard layering for silent callback paths

## Files Changed

- `crates/mika-agent/src/agent.rs` — `LoopMode::Silent { max_steps }`, `SilentTrigger::max_steps()`,
  `attempt_continuation_turn()` shared helper, nudge for all modes, silent continuation turn
