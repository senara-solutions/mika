# Brainstorm: Generic Workflow Task Tracking (Work Items)

**Date:** 2026-03-11
**Status:** Complete
**Author:** Sami / Claude

## What We're Building

A first-class "work item" concept in Mika that lets the agent track any piece of work — with subtasks, external references, status progression, and audit trail — regardless of how the work gets done.

**The gap today:** Mika has time-based reminders (recurring/one-shot) and internal callback tasks (created by long-running exec handlers). But the agent cannot create a trackable work item that represents "a thing to be done," link it to external references (GitHub issues, URLs), or manually progress its status.

### Use Cases

- "Implement GitHub issue #123" — fetch issue, create work item, trigger self-dev
- "Research competitor pricing" — create work item, trigger team run
- "Review the Q3 deck before Thursday" — manual work item, Mika tracks and reminds
- "Wait for Sarah's reply about the contract" — work item blocked on external input
- "Prepare morning briefing data" — work item, triggers multiple skills
- Claude-asked questions during self-dev linked back to the originating work item

## Why This Approach

Work items are **tasks with `trigger_type: 'manual'`**. No new tables — just new columns on the existing `tasks` table and a new trigger type. The task engine ignores manual tasks (fully inert, like callbacks are event-driven). The agent decides when to create, progress, and complete work items.

This approach was chosen because:
1. **Builds on existing infrastructure** — task engine, parent_task_id linkage, audit_events, unified_timeline all work unchanged
2. **Clean layering** — `create_work_item` (agent-facing tool with guards) wraps `create_task` (internal engine function with no policy)
3. **No new scheduling complexity** — the tick loop stays clean (time-based and recurring only). Staleness detection is the heartbeat's job

## Key Decisions

### 1. Tool Layering — Agent-Facing Wrapper Over Internal Engine

`create_task` remains the internal engine function — creates any task type (callback, recurring, manual). Stays internal, not agent-facing.

`create_work_item` is the agent-facing tool that wraps `create_task` with:
- Hardcoded `trigger_type: 'manual'`
- All 5 loop prevention guards (code-level enforcement)
- Audit event logging
- The new fields (`reference_url`, `source`)

The guards live in the agent-facing layer, not the engine.

### 2. Status Model — Add `blocked`, Free Transitions

Add `blocked` to the tasks status CHECK constraint. Clear semantics: "I can't proceed until external input arrives." Queryable: "show me all blocked work items" is a useful dashboard filter.

No state machine for transitions. Any status → any status is allowed. Every transition is logged as an `audit_event`. The audit log IS the state machine — it records what happened. The agent's judgment is trusted; bad patterns are fixed via prompt tuning, not code constraints.

### 3. References — Single URL Column

One `reference_url TEXT` column. The primary reference is the thing that triggered the work. Related links go in the task description or notes. We're not building Jira.

### 4. Transition Notes — Audit Events Only

Status transition notes go in `audit_event.reasoning`. No new column on tasks. The task table shows current state; the audit log shows history. Orthogonal design.

### 5. Tick Loop — Fully Inert for Manual Tasks

The tick loop ignores `trigger_type = 'manual'` entirely. Work items are passive tracking records. The heartbeat already evaluates pending commitments — if a work item is stale, the heartbeat's job is to notice and nudge.

### 6. Query Tool — Dedicated `list_work_items`

"What am I tracking?" is the most basic question. A dedicated tool with status/source filters returns exactly what's needed in one call. `query_timeline` is for cross-subsystem investigation; `list_work_items` is for daily work management.

### 7. Cap Enforcement — Per-Session DB Query

Guard #5 (agent-created work items capped at 5 per session) enforced via `SELECT COUNT(*)` with `session_id` and `trigger_type = 'manual'`. Negligible cost for a handful of creations. No new state on ToolContext.

## Schema Changes (v7 → v8)

**New columns on `tasks`:**
- `reference_url TEXT` — nullable, link to GitHub issue, Slack thread, document
- `source TEXT` — nullable, origin: 'user_request', 'github_issue', 'team_run', 'self_dev'

**Updated CHECK constraints:**
- `trigger_type` — add `'manual'` to allowed values
- `status` — add `'blocked'` to allowed values

**Migration:** Idempotent v7→v8 with per-step existence checks (established pattern).

## New/Updated Tools

### `create_work_item` (new, agent-facing)
- **Inputs:** `label` (required), `reference_url` (optional), `source` (optional), `parent_task_id` (optional)
- **Output:** task_id
- **Internals:** Creates task via `db.create_task()` with `trigger_type: 'manual'`, `action_type: 'resume_agent'` (or a new no-op action), `status: 'pending'`
- **Guards:** All 5 loop prevention guards (see below)
- **Audit:** Logs `audit_event` with `tool_name: 'create_work_item'`, `target_key: 'task:{id}'`

### `update_task_status` (new, agent-facing)
- **Inputs:** `task_id` (required), `status` (required: in_progress/blocked/completed/cancelled), `note` (optional)
- **Output:** confirmation with old → new status
- **Validation:** Task must exist and be `trigger_type: 'manual'` (agent can't change callback/recurring task status via this tool)
- **Audit:** Logs `audit_event` with `tool_name: 'update_task_status'`, `target_key: 'task:{id}'`, `before_value: old_status`, `after_value: new_status`, `reasoning: note`

### `list_work_items` (new, agent-facing)
- **Inputs:** `status` (optional filter), `source` (optional filter), `include_children` (optional bool)
- **Output:** list of work items with id, label, status, source, reference_url, created_at, child count

### Updates to existing components:
- **self-dev skill:** Accept `parent_task_id` — links the Claude Code session as a child task
- **`mika ask` CLI:** Add `--parent-task` flag — metadata threading for claude-asked relay

## Loop Prevention Guards (All Code-Level)

1. **No top-level creation from task context.** If processing within task abc-123, can create CHILD tasks (with `parent_task_id`) but NOT new top-level work items. Only user messages or unprompted agent turns can create top-level work items.

2. **Depth cap of 3 levels.** Enforced by existing DB CHECK constraint `depth BETWEEN 0 AND 3`. Work item (0) → child self-dev (1) → grandchild long_running (2). No deeper.

3. **Callback/claude-asked turns are guarded.** `is_callback_turn`-style guard blocks new task creation and long_running tools. Answer and return.

4. **One active self-dev per work item.** Before creating a self-dev child task, query for existing pending/in_progress children of the same parent with `action_type = 'run_skill'` and self-dev label. Block if found.

5. **5 agent-created work items per session.** `SELECT COUNT(*) FROM tasks WHERE created_by_session = ? AND trigger_type = 'manual' AND source != 'user_request'`. Reject if >= 5.

Prompt guidance added as defense in depth only.

## Ideal Flow

```
User: "Implement https://github.com/senara-solutions/mika/issues/123"

Mika:
1. github skill → fetch issue title, body, labels
2. create_work_item(
     label: "Implement #123: Add mention_count to people table",
     source: "github_issue",
     reference_url: "https://github.com/.../issues/123"
   ) → task abc-123
3. self-dev skill(
     goal: "implement the feature described in #123",
     parent_task_id: abc-123
   ) → long_running, child task of abc-123
4. Claude Code works in tmux
5. Claude-asked question arrives:
   mika ask --parent-task abc-123 "[claude-asked] Should I use COLLATE NOCASE?"
   → Mika answers (guarded: no new tasks)
   → Answer relayed back to Claude Code
6. Claude Code completes → callback → child task done
7. Mika: "Feature #123 implemented. PR ready for review."
   → parent task abc-123 marked completed
```

## Observability

- All tasks in the tree share trace lineage via `unified_timeline`
- Work item creation logged as `audit_event`
- Status transitions logged as `audit_events` with `trace_id`
- Dashboard: work items as expandable trees with children
- Investigation agent can answer "What happened during work item abc-123?" by querying full task tree + linked messages + audit_events
- `reference_url` rendered as clickable link in dashboard

## Constraints

- No new tables — columns added to existing `tasks` table
- Manual tasks are fully inert in the tick loop
- Existing reminder and callback flows unchanged
- Agent decides when to complete — no auto-completion for manual tasks
- All loop prevention guards are code-level, not prompt-level

## Open Questions

None — all design questions resolved during brainstorming.
