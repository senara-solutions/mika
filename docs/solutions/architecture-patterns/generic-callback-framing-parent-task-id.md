---
title: Generic callback framing with parent_task_id surfacing
category: architecture-patterns
date: 2026-03-29
tags: [callback, parent_task_id, framing, single-source-of-truth, skill-driven]
issue: "#313"
module: agent, task_engine, cli
---

# Generic Callback Framing with parent_task_id Surfacing

## Problem

Two issues with callback framing in the agent engine:

1. **Missing parent_task_id:** After a long-running callback completes, the agent receives the callback task's own UUID but not the parent work item ID. The `parent_task_id` field exists on the `Task` struct at both dispatch points (server dispatcher and TUI poller) but was dropped when constructing `SilentTrigger::Callback` and `AgentRequest::CallbackResult`. This forced the agent to remember work item IDs across async gaps — unreliable after conversation compaction.

2. **Competing instruction sets:** `build_callback_trigger_context()` had 3-branch routing: claude-pilot success (5-step workflow), claude-pilot failure (escalation), and generic. The engine's claude-pilot-specific instructions competed with the self-dev skill's 450-line prompt, creating two sources of truth. Weaker models followed neither fully.

## Root Cause

Data threading gap: `parent_task_id` was available in the `Task` struct but not propagated through the intermediate types (`SilentTrigger::Callback`, `AgentRequest::CallbackResult`) to the framing functions.

Architecture anti-pattern: the engine encoded workflow-specific logic (`CLAUDE_PILOT_CALLBACK_LABEL` detection) that belonged in the skill layer.

## Solution

### A. Simplify to generic framing

Removed `CLAUDE_PILOT_CALLBACK_LABEL` constant and 3-branch routing. All callbacks get the same generic framing:

```rust
pub fn build_callback_trigger_context(
    label: &str,
    task_id: &str,
    parent_task_id: Option<&str>,
    result: &str,
    failed: bool,
) -> String {
    let base = format_callback_framing(label, task_id, parent_task_id, result, failed);
    format!(
        "{base}\n\
         IMPORTANT: A successful result confirms only the specific action performed. \
         NEVER extrapolate to downstream states ...\n\n\
         Follow the workflow defined by your active skills for this callback type. \
         If no skill-specific workflow applies, use send_message to notify the user \
         with a clear, concise summary of the key findings and any recommended actions."
    )
}
```

### B. Thread parent_task_id through both paths

1. Added `parent_task_id: Option<String>` to `SilentTrigger::Callback` variant
2. Added `parent_task_id: Option<String>` to `AgentRequest::CallbackResult` variant
3. Server path: `dispatcher.rs` reads `task.parent_task_id.clone()` into the trigger
4. TUI path: `app.rs` reads `task.parent_task_id` into the request, `chat.rs` passes it through
5. `format_callback_framing()` emits `Parent work item: {id}` when parent is set

### Files changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Remove `CLAUDE_PILOT_CALLBACK_LABEL`, simplify `build_callback_trigger_context()`, add `parent_task_id` to `format_callback_framing()` and `SilentTrigger::Callback` |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | Thread `task.parent_task_id` into `SilentTrigger::Callback` |
| `crates/mika-cli/src/tui/app.rs` | Add `parent_task_id` to `AgentRequest::CallbackResult`, forward from `poll_callback_tasks()` |
| `crates/mika-cli/src/commands/chat.rs` | Destructure and pass `parent_task_id` to framing, include in metadata JSON |

## Prevention

- **Follow the `Option<String>` field propagation pattern** established by `trace_id`: add field to intermediate types, thread from callers, use `None` for backward compatibility. After implementation, grep for `parent_task_id: None` at call sites where context is available — any such site is a propagation bug.
- **Engine should not encode workflow-specific logic.** If a callback needs special handling, that logic belongs in the skill prompt (single source of truth), not in the engine's framing function.
- **Dual-path consistency:** Both callback delivery paths (server/silent and TUI) must call the same public function with the same parameters. Per `tui-callback-skips-mika-qa-delegation.md`, `build_callback_trigger_context()` is the single entry point.

## Cross-references

- #314: Callback turn work item context injection (companion, already merged)
- `docs/solutions/logic-errors/tui-callback-skips-mika-qa-delegation.md`: Dual-path consistency pattern
- `docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md`: Option<String> propagation pattern
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`: Callback safety constraints
