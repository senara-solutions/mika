---
title: "Task health awareness: heartbeat prompt injection for anomaly detection"
category: architecture-patterns
date: 2026-03-25
tags: [heartbeat, prompt, task-engine, silent-mode, anomaly-detection, preferences]
module: agent, db, prompt, task_engine
issue: 117
---

# Task Health Awareness: Heartbeat Prompt Injection for Anomaly Detection

## Problem

Mika's hourly heartbeat runs only surfaced active manual work items via a flat `<pending-work-items>` block. Anomalous task states across all trigger types — stuck callbacks, failed recurring tasks, stale blocked items, long-running tasks — were invisible to the agent. The agent also lacked self-knowledge about task lifecycles (trigger types, status transitions) and could not answer user questions about her own task system accurately.

## Root Cause

The heartbeat prompt injection was limited to a single query (`list_active_work_items`) filtering only `trigger_type = 'manual'` with `status IN ('pending', 'in_progress', 'blocked')`. No cross-trigger-type health check existed. Task lifecycle knowledge was not part of the `get_documentation` corpus.

## Solution

### Phase 1: Self-Knowledge Documentation

Added `docs/task-system.md` as a new `get_documentation` topic following the proven 5-step checklist (`docs/solutions/integration-issues/adding-get-documentation-topic.md`):

1. `build.rs` — added `"task-system.md"` to `DOCS` array
2. `builtin_handlers.rs` — added `static DOC_TASK_SYSTEM`, match arm, error message update
3. `self-knowledge/system_prompt.md` — added topic description
4. `self-knowledge/tools.json` — added `"task-system"` to enum
5. Test array — added `"task-system"` to `test_get_documentation_all_embedded_topics`

### Phase 2: Heartbeat Task Health Context

Replaced `pending_work_items: &[Task]` on `SilentPromptContext` with `task_health: Option<&TaskHealthSummary>` + `stored_preferences: &[Preference]`.

**Key design decisions:**

- **Unified `<task-health>` block** replaces `<pending-work-items>` — includes both active work items AND anomalies in one section, preventing token waste from redundant blocks.
- **5 anomaly types** with hardcoded threshold constants in `task_engine::types::health_thresholds`:
  - `stuck_callback` — callback `completed` but not `delivered` for >10min
  - `failed_recurring` — recurring task `failed` in last 24h
  - `long_running` — task `in_progress` for >1h (non-manual)
  - `stale_blocked` — manual work item `blocked` with no activity >24h
  - `github_linked` — active work item with GitHub reference URL
- **Anomaly cap** at 10 (`MAX_ANOMALIES`) with priority ordering: stuck > failed > long > stale > github
- **Heartbeat and callback gating** — task health and preferences loaded for `SilentTrigger::Heartbeat` and `SilentTrigger::Callback`, not reflection/skill_run triggers. Callback turns benefit from work item context for correlating results to in-flight work items (#314)
- **Preferences filtered** to `task_policy_*` prefix via `search_preferences("task_policy_")`

**Helper extraction:** The 5 anomaly queries share a `query_anomalies` closure that handles row-mapping, iteration, and struct construction — eliminating ~130 lines of duplicated code.

### Phase 3: Preference-Driven Autonomous Actions

Stored preferences injected as `<stored-preferences>` block with `<task-health-instructions>` guiding the agent to check preferences before proposing actions, and to offer preference storage after user-confirmed actions.

## Prevention / Best Practices

1. **Follow the `<pending-work-items>` injection pattern** for any new structured data in heartbeat prompts: sanitize labels (200-char truncation, strip `<>` and newlines), cap result counts, use XML-tagged blocks.

2. **Gate task health data to heartbeat and callback triggers only** — don't inject into reflection/skill_run prompts where the agent has a different job. Callback turns benefit from work item context for correlating results to in-flight work items (#314).

3. **Filter injected preferences by purpose** — don't dump all preferences into every prompt. Use `search_preferences("prefix_")` to scope to the relevant category.

4. **Use named threshold constants** in `task_engine::types::health_thresholds` — don't hardcode magic numbers in queries. This makes future configurability easy.

5. **The `get_documentation` topic checklist** (`docs/solutions/integration-issues/adding-get-documentation-topic.md`) must be followed exactly — missing any of the 5 sync points causes build failures or silent unavailability.

## Key Files

- `docs/task-system.md` — authoritative task lifecycle reference
- `crates/mika-agent/src/db.rs` — `TaskHealthSummary`, `TaskHealthAnomaly`, `get_task_health_summary()`
- `crates/mika-agent/src/prompt.rs` — `SilentPromptContext`, `build_silent_prompt()`, `sanitize_label()`
- `crates/mika-agent/src/agent.rs` — heartbeat-gated loading in `run_silent_agent()`
- `crates/mika-agent/src/task_engine/types.rs` — `health_thresholds` constants

## Related

- `docs/solutions/integration-issues/adding-get-documentation-topic.md` — checklist followed for Phase 1
- `docs/solutions/architecture-patterns/work-item-tracking-manual-task-reuse.md` — original `<pending-work-items>` pattern
- `docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md` — lesson on checking both completed AND failed states
