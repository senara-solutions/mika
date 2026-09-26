---
title: "feat: inject active work items into callback turn context"
type: feat
status: completed
date: 2026-03-29
issue: 314
---

# feat: inject active work items into callback turn context

## Overview

Callback turns (via `SilentTrigger::Callback`) currently receive no context about active work items — task health injection is gated to `SilentTrigger::Heartbeat` only. After a long-running background task (e.g., a 10-minute claude-pilot run), the callback agent has zero awareness of in-flight work items unless it explicitly queries `list_work_items`. This forces the agent to rely on conversation memory, which is unreliable after async gaps — especially for models without prompt caching.

## Proposed Solution

Expand the `matches!()` guard in `run_silent_agent()` (agent.rs line ~1780) to include `SilentTrigger::Callback { .. }` alongside `SilentTrigger::Heartbeat`. This gives callback turns the same `<active-work-items>`, `<task-health>` anomalies, and `<stored-preferences>` blocks that heartbeat turns already receive.

## Technical Approach

### Code Change (agent.rs)

Single guard expansion in `run_silent_agent()`:

```rust
// Before
let (task_health, stored_preferences) = if matches!(&params.trigger, SilentTrigger::Heartbeat) {

// After
let (task_health, stored_preferences) = if matches!(
    &params.trigger,
    SilentTrigger::Heartbeat | SilentTrigger::Callback { .. }
) {
```

### What This Enables

1. **`<active-work-items>`** — list of pending/in_progress/blocked manual work items with IDs, labels, statuses, ages, and reference URLs
2. **`<task-health>` anomalies** — stuck callbacks, failed recurring tasks, long-running tasks, stale blocked items
3. **`<stored-preferences>`** — `task_policy_*` preferences for autonomous action

### What Stays Excluded

- `SilentTrigger::Reflection` — different prompt budget, focused on memory consolidation
- `SilentTrigger::SkillRun` — focused on executing a specific skill

### Scope Boundaries

- **TUI callback path** (`run_agent()` in `chat.rs`) uses the conversation prompt builder which has no `task_health` field. This asymmetry is accepted — TUI users have full conversation history context and can interactively query work items. Server-side callbacks (the primary autonomous path) are the target.
- **No prompt.rs changes** — `build_silent_prompt()` already handles `Some(health)` generically.
- **No db.rs changes** — `get_task_health_summary()` and `list_active_work_items()` are reused as-is.

## Acceptance Criteria

- [x] Callback turns receive `<active-work-items>` context in the system prompt
- [x] Callback turns receive `<task-health>` anomalies
- [x] Callback turns receive stored `task_policy_*` preferences
- [x] No regression on heartbeat behavior
- [x] Reflection and SkillRun triggers remain excluded
- [x] Tests verify callback turns receive task health data
- [x] Tests verify Reflection/SkillRun triggers do NOT receive task health data

## Test Plan

### Unit tests (agent.rs)

1. **New test: callback trigger receives task health** — construct `SilentAgentParams` with `SilentTrigger::Callback`, verify the match guard populates `task_health` and `stored_preferences` (or test at the prompt level that the rendered prompt contains `<active-work-items>`)
2. **New test: reflection trigger does NOT receive task health** — verify `SilentTrigger::Reflection` still gets `(None, vec![])`
3. **New test: skill_run trigger does NOT receive task health** — verify `SilentTrigger::SkillRun` still gets `(None, vec![])`
4. **Existing tests pass** — all heartbeat and callback tests in agent.rs and prompt.rs

### Files

- `crates/mika-agent/src/agent.rs` — the `matches!()` guard (~line 1780), plus new tests
- `crates/mika-agent/src/prompt.rs` — no changes needed (existing formatting handles `Some(health)` generically)

## Sources

- Issue: #314
- Task health injection pattern: `docs/solutions/architecture-patterns/task-health-awareness-heartbeat-injection.md`
- Callback lifecycle: `docs/solutions/architecture/callback-resume-agent-lifecycle.md`
- Callback loop prevention: `docs/solutions/architecture-patterns/callback-task-loop-prevention.md`
