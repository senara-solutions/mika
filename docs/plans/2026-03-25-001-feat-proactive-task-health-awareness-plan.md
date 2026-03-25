---
title: "feat: Add proactive task health awareness and self-knowledge of task lifecycles"
type: feat
status: completed
date: 2026-03-25
issue: 117
---

# Add Proactive Task Health Awareness and Self-Knowledge of Task Lifecycles

## Overview

Mika lacks self-knowledge about her own task system lifecycle and has no proactive reasoning about task health during periodic heartbeat check-ins. This feature closes two gaps:

1. **Self-knowledge gap:** Mika cannot answer questions about her own task types, status transitions, or trigger types — this knowledge isn't part of her documentation corpus.
2. **Proactive task health gap:** Heartbeat runs only surface active manual work items (`<pending-work-items>`). Anomalous states across all trigger types (stuck callbacks, failed recurring tasks, stale blocked items, long-running tasks) are invisible.

The feature ships in three phases: documentation (Phase 1), heartbeat health injection (Phase 2), and preference-driven autonomous actions (Phase 3).

## Problem Statement

When a user asks "what types of tasks do you track?" or "what status transitions exist?", the agent hallucinates or gives incomplete answers because no authoritative documentation exists in the `get_documentation` corpus. During hourly heartbeat runs, the agent sees active manual work items but has zero visibility into callback tasks stuck at `completed` (never delivered), recurring tasks that have `failed`, blocked work items with no activity for days, or tasks running far beyond their expected duration. The agent cannot proactively notify the user about these anomalies.

## Proposed Solution

### Phase 1: Self-Knowledge Documentation

Add a `docs/task-system.md` document covering trigger types, action types, status definitions, and per-trigger-type status transition diagrams. Wire it into `get_documentation` following the proven 5-step checklist (see `docs/solutions/integration-issues/adding-get-documentation-topic.md`).

**Files to modify:**

| File | Change |
|------|--------|
| `docs/task-system.md` | **New file** — authoritative task lifecycle reference |
| `crates/mika-agent/build.rs` | Add `"task-system.md"` to `DOCS` array (alphabetically) |
| `crates/mika-agent/src/skills/builtin_handlers.rs` | Add `static DOC_TASK_SYSTEM` include_str!, match arm in `get_documentation()`, update error message |
| `crates/mika-agent/templates/skills/self-knowledge/system_prompt.md` | Add `task-system` topic with description |
| `crates/mika-agent/templates/skills/self-knowledge/tools.json` | Add `"task-system"` to topic enum array |
| `crates/mika-agent/docs/task-system.md` | **New file** — crate-local fallback copy for crates.io |

**Documentation content outline:**

```markdown
# Task System

## Trigger Types
- time, recurring, callback, user_reply, event, condition, manual, a2a
- Purpose and lifecycle per type

## Action Types
- send_message, run_skill, inject_context, resume_agent, invoke_orchestrator, none

## Status Definitions
- pending, recurring_active, in_progress, blocked, completed, delivered, failed, expired, cancelled
- Terminal vs non-terminal classification

## Status Transition Diagrams
- Per trigger type (time, recurring, callback, manual, user_reply/event/condition)
- ASCII diagrams matching the issue specification

## Anomaly Definitions
- Callback stuck at completed (not delivered) for >10 minutes
- Task in_progress longer than estimated_duration_secs (or 1 hour default)
- Blocked work item with no updated_at change in >24 hours
- Failed recurring task (should be cycling)
```

### Phase 2: Heartbeat Task Health Context

Add a DB query that detects anomalous task states and inject the results into the heartbeat prompt as a unified `<task-health>` block that **replaces** the existing `<pending-work-items>` block (the active work items are included in the new block alongside anomalies, preventing token waste from redundant sections).

#### 2a: TaskHealthSummary Struct and DB Query

**New struct** in `crates/mika-agent/src/db.rs`:

```rust
pub struct TaskHealthAnomaly {
    pub task_id: String,
    pub label: String,
    pub trigger_type: String,
    pub status: String,
    pub anomaly_type: String,        // "stuck_callback", "stale_blocked", "failed_recurring", "long_running", "github_linked"
    pub age_description: String,     // "3h 22m", "5 days", etc.
    pub reference_url: Option<String>,
}

pub struct TaskHealthSummary {
    pub active_work_items: Vec<Task>,       // existing list_active_work_items data
    pub anomalies: Vec<TaskHealthAnomaly>,  // capped at 10
}
```

**New DB method** `get_task_health_summary(agent_id: &str) -> Result<TaskHealthSummary>`:

Anomaly detection queries with hardcoded thresholds (named constants):

| Anomaly Type | Query Condition | Threshold |
|---|---|---|
| `stuck_callback` | `trigger_type='callback' AND status='completed' AND updated_at < threshold` | 10 minutes (`STUCK_CALLBACK_THRESHOLD_SECS = 600`) |
| `stale_blocked` | `trigger_type='manual' AND status='blocked' AND updated_at < threshold` | 24 hours (`STALE_BLOCKED_THRESHOLD_SECS = 86400`) |
| `failed_recurring` | `trigger_type='recurring' AND status='failed'` | Any failed recurring in last 24h |
| `long_running` | `status='in_progress' AND fired_at < threshold` | 1 hour (`LONG_RUNNING_THRESHOLD_SECS = 3600`) or `estimated_duration_secs * 2` if set |
| `github_linked` | `trigger_type='manual' AND status IN ('pending','in_progress') AND reference_url LIKE '%github.com%'` | No time threshold — presence of URL signals the agent should inspect |

All queries filter `WHERE agent_id = ?` — agent-scoped by design. Total anomalies capped at 10, sorted by: stuck_callback > failed_recurring > long_running > stale_blocked > github_linked.

**Async wrapper** in `crates/mika-agent/src/async_db.rs`: `get_task_health_summary(&self) -> Result<TaskHealthSummary>`.

#### 2b: Prompt Assembly Changes

**Modify `SilentPromptContext`** in `crates/mika-agent/src/prompt.rs`:

```rust
// Replace:
pub pending_work_items: &'a [Task],

// With:
pub task_health: Option<TaskHealthSummary>,
```

**Modify `build_silent_prompt()`** — replace the `<pending-work-items>` block (lines 562–588) with a unified `<task-health>` block:

```xml
<task-health>
<active-work-items>
- [in_progress] abc-123: Implement auth flow (age: 3d, ref: github.com/org/repo/issues/42)
- [blocked] def-456: Design review (age: 5d)
</active-work-items>

<anomalies>
- [stuck_callback] ghi-789: Build deployment (stuck 3h 22m — completed but never delivered)
- [failed_recurring] jkl-012: Heartbeat (failed 45m ago — recurring task stopped cycling)
- [github_linked] abc-123: Implement auth flow (has linked GitHub PR — use check_work_item to inspect)
</anomalies>

<task-health-instructions>
Review the task health summary above. For each anomaly:
1. If a stored preference covers this action pattern, take it autonomously and include "(per your standing preference)" in any notification.
2. Otherwise, notify the user via send_message with the anomaly details and suggest an action.
3. For github_linked items, call check_work_item to inspect the linked PR/issue status before notifying.
4. Include the task ID in all notifications so the user can reference it.
5. Present findings as "as of this check" — task states may have changed since the query ran.
</task-health-instructions>
</task-health>
```

Labels sanitized with existing pattern: 200-char truncation, `<>` stripping. Reference URLs also sanitized.

**Modify `run_silent_agent()`** in `crates/mika-agent/src/agent.rs` — replace `list_active_work_items()` call with `get_task_health_summary()`:

```rust
// Replace:
let pending_work_items = db.list_active_work_items().await.unwrap_or_default();

// With:
let task_health = db.get_task_health_summary().await.ok();
```

#### 2c: Agent Scoping Reinforcement

Add a brief agent-scoping instruction to the `<task-health-instructions>` block: "You can only see and act on your own tasks. Never attempt to query or modify tasks belonging to other agents."

This reinforces the DB-level `WHERE agent_id = ?` guard with a prompt-level instruction (defense-in-depth per `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`).

### Phase 3: Preference-Driven Autonomous Actions

Enhance the heartbeat prompt to include stored preferences and instruct the agent to check them before proposing actions.

#### 3a: Preferences in Heartbeat Prompt

**Extend `SilentPromptContext`** with:

```rust
pub stored_preferences: &'a [Preference],
```

**Add `<stored-preferences>` block** in `build_silent_prompt()` (after `<task-health>`, before trigger context):

```xml
<stored-preferences>
- task_policy_merged_pr: When a work item's linked PR is merged, mark it completed automatically
- task_policy_stale_blocked: Notify user when blocked items have no activity for 7+ days
</stored-preferences>
```

Conditional on non-empty. Category names and values sanitized with the same 200-char truncation pattern.

**Modify `run_silent_agent()`** to fetch preferences:

```rust
let stored_preferences = db.list_preferences().await.unwrap_or_default();
```

#### 3b: Preference Learning Instructions

Add to `<task-health-instructions>`:

```
6. After taking a corrective action that the user confirmed, ask: "Should I always do this automatically in the future? I can remember this as a standing preference."
7. If the user confirms, store the policy via store_fact with category "preference" and a key prefixed with "task_policy_" (e.g., task_policy_merged_pr_auto_complete).
```

This is prompt-only — no new code beyond the preference injection in the prompt.

## Technical Considerations

### Architecture Impacts

- **No schema changes.** All data lives in existing `tasks` and `preferences` tables.
- **No new tools.** Phase 2 uses existing tools (`send_message`, `check_work_item`, `update_work_item_status`). Phase 1 extends `get_documentation` (existing builtin handler).
- **Prompt size budget.** The `<task-health>` block replaces `<pending-work-items>` (net token impact depends on anomaly count). Cap at 10 anomalies + 10 active work items. Worst case adds ~800 tokens to the heartbeat prompt.
- **User-triggered review.** No new tool needed — the system prompt and self-knowledge skill already guide the agent to use `list_work_items`, `check_work_item`, and `query_timeline` for comprehensive reviews. The new `task-system` documentation topic gives the agent authoritative lifecycle knowledge to reason about what it finds.

### Performance Implications

- The `get_task_health_summary()` query runs once per heartbeat (hourly). Five SQL queries (one per anomaly type) plus the existing `list_active_work_items` query. Each is simple index-scanned with `WHERE agent_id = ?`. Expected latency: <5ms total.
- No new indexes needed — existing `idx_tasks_agent_status` and `idx_tasks_manual_active` cover all query patterns.

### Security Considerations

- Agent scoping enforced at DB level (`WHERE agent_id = ?`) and reinforced in prompt.
- Label/URL sanitization prevents prompt injection (existing pattern reused).
- No new external API calls from the DB query layer — GitHub enrichment deferred to agent tool use.
- Preferences are agent-scoped (`(agent_id, category)` PK).

## System-Wide Impact

### Interaction Graph

1. Heartbeat fires → `dispatch_heartbeat()` → `run_silent_agent()` → `get_task_health_summary()` (new) → injects `<task-health>` into prompt → agent reasons → may call `send_message` / `check_work_item` / `update_work_item_status`.
2. User asks about tasks → agent calls `get_documentation("task-system")` → builtin handler returns embedded doc.
3. User asks to check task health → agent uses existing tools (`list_work_items`, `check_work_item`, `query_timeline`) informed by `task-system` self-knowledge.

### Error Propagation

- `get_task_health_summary()` failure → `unwrap_or_default()` → empty health summary → heartbeat proceeds without task health (graceful degradation).
- `get_documentation("task-system")` failure → returns error message listing valid topics → agent cannot answer but does not crash.
- Preference fetch failure → `unwrap_or_default()` → empty preferences → agent proposes actions without checking policies.

### State Lifecycle Risks

- Race between health check query and task state change: mitigated by prompt instruction ("present findings as of this check") and existing terminal-state guards on `update_work_item_status`.
- No new state mutations from the health check query itself — all state changes go through existing tool layer.

### API Surface Parity

- `get_documentation` topic list must be updated in 5 files (build.rs, builtin_handlers.rs, self-knowledge prompt, tools.json, error message). The existing checklist at `docs/solutions/integration-issues/adding-get-documentation-topic.md` covers this.
- `SilentPromptContext` field change requires updating all call sites (one: `run_silent_agent`).
- Removing `pending_work_items` from `SilentPromptContext` requires updating all existing prompt tests that reference it.

### Integration Test Scenarios

1. **Heartbeat with anomalies:** Create tasks in various anomalous states → fire heartbeat → verify `<task-health>` block contains expected anomalies with correct labels.
2. **Heartbeat with no anomalies:** Clean task state → fire heartbeat → verify `<task-health>` block shows active work items only, no anomalies section.
3. **get_documentation("task-system"):** Call handler → verify returns non-empty, non-error content containing trigger type definitions.
4. **Agent scoping:** Create tasks for two different agent_ids → query health for agent A → verify agent B's tasks are absent.
5. **Threshold boundary:** Create a callback task completed exactly at the threshold boundary → verify it does/does not appear as an anomaly.

## Acceptance Criteria

### Phase 1: Self-Knowledge Documentation

- [x] `docs/task-system.md` exists with trigger types, action types, status definitions, and transition diagrams
- [x] `get_documentation("task-system")` returns the document content (not an error)
- [x] Self-knowledge skill's `system_prompt.md` lists `task-system` topic
- [x] Self-knowledge skill's `tools.json` includes `"task-system"` in the topic enum
- [x] `test_get_documentation_all_embedded_topics` test passes with `"task-system"` included
- [x] Error message for invalid topics includes `task-system`
- [x] Crate-local fallback at `crates/mika-agent/docs/task-system.md` exists

### Phase 2: Heartbeat Task Health Context

- [x] `TaskHealthAnomaly` and `TaskHealthSummary` structs defined in `db.rs`
- [x] `get_task_health_summary(agent_id)` DB method returns anomalies matching threshold constants
- [x] Async wrapper `get_task_health_summary()` exists in `async_db.rs`
- [x] `SilentPromptContext` uses `task_health: Option<TaskHealthSummary>` (replaces `pending_work_items`)
- [x] `build_silent_prompt()` renders `<task-health>` block with active work items and anomalies
- [x] `<task-health-instructions>` block guides agent behavior
- [x] `run_silent_agent()` calls `get_task_health_summary()` instead of `list_active_work_items()`
- [x] Agent-scoping reinforcement present in prompt instructions
- [x] Labels and URLs sanitized with 200-char truncation and `<>` stripping
- [x] Anomalies capped at 10 total
- [x] All existing silent prompt tests updated for new struct shape
- [x] Unit tests for `get_task_health_summary` with various anomalous states
- [x] Unit test for empty health summary (no anomalies)
- [x] Unit test for threshold boundary conditions

### Phase 3: Preference-Driven Autonomous Actions

- [x] `SilentPromptContext` includes `stored_preferences: &'a [Preference]`
- [x] `build_silent_prompt()` renders `<stored-preferences>` block when non-empty
- [x] `run_silent_agent()` fetches preferences via `list_preferences()`
- [x] `<task-health-instructions>` includes preference-checking and preference-learning guidance
- [x] Preferences sanitized with same truncation pattern

## Implementation Phases

### Phase 1: Self-Knowledge Documentation

**Effort:** Small — well-defined 5-step checklist, no architectural decisions.

1. Write `docs/task-system.md` with full lifecycle reference
2. Add to `build.rs` DOCS array
3. Add `include_str!`, match arm, and error message update in `builtin_handlers.rs`
4. Update `self-knowledge/system_prompt.md` and `tools.json`
5. Add `"task-system"` to `test_get_documentation_all_embedded_topics` test array
6. Copy to `crates/mika-agent/docs/task-system.md` for crates.io fallback

### Phase 2: Heartbeat Task Health Context

**Effort:** Medium — new DB query, struct, prompt changes, test updates.

1. Define threshold constants in `task_engine/types.rs`
2. Add `TaskHealthAnomaly` and `TaskHealthSummary` structs to `db.rs`
3. Implement `get_task_health_summary()` in `db.rs` with 5 anomaly queries
4. Add async wrapper in `async_db.rs`
5. Replace `pending_work_items` with `task_health` in `SilentPromptContext`
6. Replace `<pending-work-items>` with `<task-health>` in `build_silent_prompt()`
7. Update `run_silent_agent()` to call `get_task_health_summary()`
8. Update all silent prompt tests
9. Write unit tests for DB query
10. Write unit tests for prompt rendering

### Phase 3: Preference-Driven Autonomous Actions

**Effort:** Small — prompt-only changes, no new logic.

1. Add `stored_preferences` field to `SilentPromptContext`
2. Add `<stored-preferences>` block to `build_silent_prompt()`
3. Fetch preferences in `run_silent_agent()`
4. Extend `<task-health-instructions>` with preference guidance
5. Update tests

## Alternatives Considered

1. **New `task_health_check` tool:** Rejected — adds a tool to the registry when the agent already has `list_work_items`, `check_work_item`, and `query_timeline`. The heartbeat uses prompt injection (not tools) for context, and interactive mode should use existing tools.

2. **Keep `<pending-work-items>` alongside `<task-health>`:** Rejected — duplicate data wastes tokens and risks conflicting instructions. A unified block is cleaner.

3. **Configurable thresholds per customer:** Rejected for v1 — premature configurability. Named constants allow easy future extraction into config.

4. **GitHub API calls in the DB query:** Rejected — the `check_work_item` tool handles GitHub enrichment with proper error handling, token management, and timeout. The health summary flags items for inspection; the agent calls the tool.

5. **Separate always-on skill for task awareness:** Rejected — the self-knowledge skill already covers agent introspection. Adding a separate skill wastes context tokens when the knowledge is better served as a `get_documentation` topic (on-demand) and heartbeat prompt injection (periodic).

## Dependencies & Risks

- **No external dependencies.** All changes are within `mika/` crate boundaries.
- **Risk: prompt size.** Mitigated by capping anomalies at 10 and replacing (not adding to) `<pending-work-items>`.
- **Risk: noisy heartbeat.** If thresholds are too aggressive, the agent over-notifies. Mitigated by conservative defaults (10min callbacks, 24h blocked, 1h long-running).
- **Risk: heartbeat-to-interactive handoff.** Self-contained messages with task IDs let the user reference specific tasks in follow-up conversations.

## Sources & References

### Internal References

- **get_documentation checklist:** `docs/solutions/integration-issues/adding-get-documentation-topic.md`
- **Heartbeat work items injection:** `crates/mika-agent/src/prompt.rs:562-588`
- **SilentPromptContext struct:** `crates/mika-agent/src/prompt.rs:468`
- **list_active_work_items DB query:** `crates/mika-agent/src/db.rs:2622-2636`
- **run_silent_agent:** `crates/mika-agent/src/agent.rs:1555`
- **Task engine types:** `crates/mika-agent/src/task_engine/types.rs`
- **Preferences DB methods:** `crates/mika-agent/src/db.rs:3571-3620`
- **Callback lifecycle:** `docs/solutions/architecture/callback-resume-agent-lifecycle.md`
- **Work item tracking:** `docs/solutions/architecture-patterns/work-item-tracking-manual-task-reuse.md`
- **Background agent checklist:** `docs/solutions/code-review-patterns/background-agent-mode-design-checklist.md`
- **Failed callbacks lesson:** `docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md`

### Related Work

- Issue: #117
- Existing `<pending-work-items>` injection (PRs #257, #258)
- Callback race condition fix (PR #264)
- Callback result truncation (PR #259)
