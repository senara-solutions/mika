---
title: "Callback turn work item context injection"
category: architecture-patterns
date: 2026-03-29
tags: [silent-agent, callback, task-health, work-items, SilentTrigger, prompt-context]
related_issues: [314]
modules: [agent, prompt, db]
---

# Callback turn work item context injection

## Problem

Callback turns (`SilentTrigger::Callback`) received no context about active work items. The task health injection guard in `run_silent_inner()` was gated to `SilentTrigger::Heartbeat` only:

```rust
let (task_health, stored_preferences) = if matches!(&params.trigger, SilentTrigger::Heartbeat) {
    // fetch task health + preferences
} else {
    (None, vec![])  // Callbacks got nothing
};
```

After a long-running background task (e.g., a 10-minute claude-pilot run), the callback agent had zero awareness of in-flight work items. It relied entirely on conversation memory to correlate the callback result to the originating work item — unreliable after async gaps, especially for models without prompt caching.

## Root cause

The original task health injection (introduced for heartbeat monitoring) used a conservative guard that excluded all non-heartbeat triggers. This was intentional at the time — the solution doc (`task-health-awareness-heartbeat-injection.md`) prevention rule #2 explicitly said "Gate heartbeat-specific data to heartbeat triggers only." However, as the callback workflow matured (workflow-aware triggers, self-dev continuation, work item tracking), the need for work item context in callback turns became clear.

## Solution

Expand the `matches!()` guard to include `SilentTrigger::Callback { .. }`:

```rust
let (task_health, stored_preferences) = if matches!(
    &params.trigger,
    SilentTrigger::Heartbeat | SilentTrigger::Callback { .. }
) {
    (
        db.get_task_health_summary().await.ok(),
        db.search_preferences("task_policy_").await.unwrap_or_default(),
    )
} else {
    (None, vec![])
};
```

This gives callback turns:
1. `<active-work-items>` — list of pending/in_progress/blocked manual work items
2. `<task-health>` anomalies — stuck callbacks, failed recurring, stale blocked items
3. `<stored-preferences>` — `task_policy_*` preferences for autonomous action

### What stays excluded

- `SilentTrigger::Reflection` — different prompt budget, focused on memory consolidation
- `SilentTrigger::SkillRun` — focused on executing a specific skill

### Safety properties preserved

- Callback agents only get `default_tools()` (read-only work item tools: `list_work_items`, `check_work_item`)
- `is_callback_turn: true` prevents spawning new long-running tasks
- `get_task_health_summary()` is a lightweight SQLite query using partial index `idx_tasks_manual_active`
- `.ok()` on the query means DB failures degrade gracefully to `(None, vec[])`

### TUI path asymmetry

The TUI callback path (`run_agent()` in `chat.rs`) uses the conversation prompt builder which has no `task_health` field. This asymmetry is intentional — TUI users have full conversation history context and can interactively query work items.

## Prevention

- When adding new `SilentTrigger`-gated context, document the inclusion/exclusion rationale for each variant — don't just gate to "heartbeat only" by default
- The `task-health-awareness-heartbeat-injection.md` solution doc prevention rule #2 has been updated to reflect callback inclusion
- Tests should cover ALL `SilentTrigger` variants (positive and negative) when modifying the guard

## Key files

- `crates/mika-agent/src/agent.rs` — `run_silent_inner()`, the `matches!()` guard (~line 1780)
- `crates/mika-agent/src/prompt.rs` — `build_silent_prompt()`, `SilentPromptContext.task_health`
- `crates/mika-agent/src/db.rs` — `get_task_health_summary()`, `list_active_work_items()`
