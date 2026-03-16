---
title: "feat: Add Tasks and Team Runs dashboard pages with cross-linking"
type: feat
status: active
date: 2026-03-16
origin: docs/brainstorms/2026-03-16-dashboard-tasks-teams-brainstorm.md
---

# feat: Add Tasks and Team Runs dashboard pages with cross-linking

## Overview

Add two new top-level dashboard pages — **Tasks** and **Team Runs** — plus deep cross-linking enhancements to existing pages. This completes the Phase 2 vision from the original dashboard brainstorm, surfacing task scheduling, team orchestration, and delegation flows for operator debugging.

## Problem Statement / Motivation

Issues #160, #161, #162 hardened trace_id correlation (schema v10–v11), but the dashboard only has 4 pages (Timeline, Agents, Sessions, Traces). Tasks and team runs are invisible unless you query SQLite directly. The data model supports full cross-subsystem correlation — the dashboard needs to surface it.

(see brainstorm: docs/brainstorms/2026-03-16-dashboard-tasks-teams-brainstorm.md)

## Proposed Solution

### Phase 1: Backend — New API Endpoints

Add two paginated list endpoints and supporting DB methods.

### Phase 2: Frontend — New Pages

Add Tasks page (4-section layout), Team Runs list page, and Team Run detail page.

### Phase 3: Cross-Linking

Enhance existing pages with bidirectional links between tasks, sessions, traces, agents, and team runs.

## Technical Approach

### Architecture

#### Task Response DTO

The `Task` struct (`crates/mika-agent/src/db.rs:109`) does not derive `Serialize`. Rather than adding `Serialize` to the full struct (which contains `process_id` and potentially large `action_config`/`input_context`/`result` fields), create a `TaskResponse` DTO in `server/dashboard.rs`:

```rust
// crates/mika-agent/src/server/dashboard.rs
#[derive(Debug, Serialize)]
pub struct TaskResponse {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub description: Option<String>,
    pub trigger_type: String,          // cron, one_shot, event, callback, manual
    pub action_type: String,           // send_message, run_skill, inject_context, resume_agent, invoke_orchestrator, none
    pub status: String,                // pending, in_progress, completed, failed, delivered, blocked, cancelled
    pub schedule: Option<String>,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub source: Option<String>,        // user_request, self_dev, etc.
    pub reference_url: Option<String>,
    pub team_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub depth: i32,
    pub created_by_session: Option<String>,
    pub created_trace_id: Option<String>,
    pub execution_trace_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    // Omitted: process_id, timeout_secs, max_retries, retry_count, input_context (large), action_config (large)
    // action_config_preview and result_preview are truncated summaries
    pub action_config_preview: Option<String>,  // first 200 chars of action_config
    pub result_preview: Option<String>,         // first 200 chars of result
}
```

Convert via `impl From<Task> for TaskResponse` with truncation.

#### API Endpoints

**`GET /api/v1/tasks`** — Paginated task list with filters:
- Query params: `status`, `trigger_type`, `action_type`, `agent_id`, `team_run_id` (use `"null"` for IS NULL, `"notnull"` for IS NOT NULL), `source`, `page`, `per_page`
- The Tasks page makes 4 calls with different filter combinations:
  - Work Items: `trigger_type=manual`
  - Team Runs section: `team_run_id=notnull` (grouped client-side by `team_run_id`)
  - Standalone Callbacks: `action_type=resume_agent,run_skill&team_run_id=null&trigger_type=callback`
  - Scheduled: `trigger_type=cron,one_shot`
- Returns `PaginatedResponse<TaskResponse>`

**`GET /api/v1/team-runs`** — Paginated team run list with filters:
- Query params: `team_name`, `status`, `from` (ISO datetime), `to` (ISO datetime), `page`, `per_page`
- Returns `PaginatedResponse<TeamRunRow>` (already derives `Serialize`)

**`GET /api/v1/tasks/:id`** — Single task detail (for future use / deep links):
- Returns `TaskResponse` or 404

#### DB Methods

```rust
// crates/mika-agent/src/db.rs — new methods

// Tasks
fn list_tasks_paginated(
    &self,
    filters: &TaskFilters,
    limit: i64,
    offset: i64,
) -> Result<Vec<Task>>

fn count_tasks(&self, filters: &TaskFilters) -> Result<i64>

fn list_tasks_paginated_with_count(
    &self,
    filters: &TaskFilters,
    limit: i64,
    offset: i64,
) -> Result<(Vec<Task>, i64)>

// Team Runs
fn list_team_runs_paginated(
    &self,
    filters: &TeamRunFilters,
    limit: i64,
    offset: i64,
) -> Result<Vec<TeamRunRow>>

fn count_team_runs(&self, filters: &TeamRunFilters) -> Result<i64>

fn list_team_runs_paginated_with_count(
    &self,
    filters: &TeamRunFilters,
    limit: i64,
    offset: i64,
) -> Result<(Vec<TeamRunRow>, i64)>

// Filter structs
pub struct TaskFilters {
    pub status: Option<String>,           // comma-separated
    pub trigger_type: Option<String>,     // comma-separated
    pub action_type: Option<String>,      // comma-separated
    pub agent_id: Option<String>,
    pub team_run_id: Option<TeamRunIdFilter>, // Null, NotNull, or Specific(String)
    pub source: Option<String>,
}

pub enum TeamRunIdFilter {
    Null,
    NotNull,
    Specific(String),
}

pub struct TeamRunFilters {
    pub team_name: Option<String>,
    pub status: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}
```

Follow the `list_sessions_paginated_with_count` pattern: count + data in one sync closure, `TASK_COLUMNS` constant for SELECT.

### Implementation Phases

#### Phase 1: Backend Endpoints (Rust)

**Files to modify:**
- `crates/mika-agent/src/db.rs` — Add `TaskFilters`, `TeamRunIdFilter`, `TeamRunFilters`, `list_tasks_paginated`, `count_tasks`, `list_tasks_paginated_with_count`, `list_team_runs_paginated`, `count_team_runs`, `list_team_runs_paginated_with_count`, `get_task_by_id` (unscoped)
- `crates/mika-agent/src/async_db.rs` — Add async wrappers for all new DB methods
- `crates/mika-agent/src/server/dashboard.rs` — Add `TaskResponse` DTO, `TasksQuery`, `TeamRunsQuery`, `handle_tasks_list`, `handle_task_detail`, `handle_team_runs_list` handlers
- `crates/mika-agent/src/server/mod.rs` — Register 3 new routes on `dashboard_routes`

**Tests:**
- `db.rs` — Roundtrip tests for `list_tasks_paginated_with_count` and `list_team_runs_paginated_with_count` with various filter combinations
- `dashboard.rs` — Integration tests for new handlers (if existing pattern exists)

**Success criteria:**
- `GET /api/v1/tasks?trigger_type=manual&page=1&per_page=20` returns paginated tasks
- `GET /api/v1/team-runs?status=running` returns paginated team runs
- `GET /api/v1/tasks/:id` returns a single task or 404
- All filters work independently and in combination

#### Phase 2: Frontend — Tasks Page

**New files:**
- `dashboard/src/api/tasks.ts` — `Task` type, `TasksFilters` interface, `useTasks(filters)` hook, `useTask(id)` hook
- `dashboard/src/pages/Tasks.tsx` — Four-section layout page
- `dashboard/src/components/TaskStatusBadge.tsx` — Status badge supporting all task/team-run statuses

**Files to modify:**
- `dashboard/src/App.tsx` — Add routes: `/tasks`, `/tasks/:id`
- `dashboard/src/components/Sidebar.tsx` — Add "Tasks" nav item with `CheckSquare` icon after "Traces"
- `dashboard/src/api/teams.ts` — Add `useTeamRuns(filters)` hook, `TeamRunsFilters` interface

**Tasks page structure:**
```
┌────────────────────────────────────────────────┐
│ Tasks                                          │
│ Manage and monitor all active tasks            │
├────────────────────────────────────────────────┤
│ ▼ Work Items                          3 active │
│ ┌──────────────────────────────────────────┐   │
│ │ ● in_progress  Prepare weekly briefing   │   │
│ │   agent: mika  source: user_request      │   │
│ │   Team Run → run-a1b2  |  2h ago         │   │
│ └──────────────────────────────────────────┘   │
│ [Pagination: 1 of 1]                           │
├────────────────────────────────────────────────┤
│ ▼ Team Runs                          2 active  │
│ ┌──────────────────────────────────────────┐   │
│ │ research-team  run-a1b2  ● running       │   │
│ │ Iteration 2/3  ← run-x9y8               │   │
│ │   ├ invoke_orchestrator  ● in_progress   │   │
│ │   ├ researcher           ● completed → S │   │
│ │   └ writer               ● in_progress→S │   │
│ └──────────────────────────────────────────┘   │
│ [Pagination: 1 of 1]                           │
├────────────────────────────────────────────────┤
│ ▼ Standalone Callbacks                       1 │
│ ┌──────────────────────────────────────────┐   │
│ │ ● pending  run_skill  mika  trace: a3f2… │   │
│ └──────────────────────────────────────────┘   │
├────────────────────────────────────────────────┤
│ ▶ Scheduled Tasks                            4 │
│   (collapsed)                                  │
└────────────────────────────────────────────────┘
```

Each section uses an independent `useTasks()` call with section-specific filters. Each section has its own pagination. The Scheduled section is collapsed by default (render header + count, lazy-load content on expand).

**TaskStatusBadge** — Static color mapping (never dynamic Tailwind classes):
```typescript
const STATUS_COLORS: Record<string, { bg: string; text: string; dot: string }> = {
  pending: { bg: 'bg-yellow-500/10', text: 'text-yellow-400', dot: 'bg-yellow-400' },
  in_progress: { bg: 'bg-blue-500/10', text: 'text-blue-400', dot: 'bg-blue-400' },
  completed: { bg: 'bg-green-500/10', text: 'text-green-400', dot: 'bg-green-400' },
  failed: { bg: 'bg-red-500/10', text: 'text-red-400', dot: 'bg-red-400' },
  delivered: { bg: 'bg-emerald-500/10', text: 'text-emerald-400', dot: 'bg-emerald-400' },
  blocked: { bg: 'bg-orange-500/10', text: 'text-orange-400', dot: 'bg-orange-400' },
  cancelled: { bg: 'bg-gray-500/10', text: 'text-gray-400', dot: 'bg-gray-400' },
  // Team run statuses
  running: { bg: 'bg-blue-500/10', text: 'text-blue-400', dot: 'bg-blue-400' },
  suspended: { bg: 'bg-amber-500/10', text: 'text-amber-400', dot: 'bg-amber-400' },
}
```

#### Phase 3: Frontend — Team Runs Pages

**New files:**
- `dashboard/src/pages/TeamRuns.tsx` — List page with filters (team name, status, date range)
- `dashboard/src/pages/TeamRunDetail.tsx` — Detail page with continuity chain, iteration breakdown, workspace, task tree

**Files to modify:**
- `dashboard/src/App.tsx` — Add routes: `/team-runs`, `/team-runs/:id`
- `dashboard/src/components/Sidebar.tsx` — Add "Team Runs" nav item with `Users` icon after "Tasks"

**Team Runs list:** Follow Sessions.tsx pattern — `useSearchParamsFilter`, filter bar, paginated table, `EmptyState` when no data.

**Team Run detail:**
- Uses 3 existing hooks: `useTeamRun(id)`, `useTeamWorkspace(id)`, new `useTeamRunSummary(id)` (wraps `/summary` endpoint — this hook needs to be added to `teams.ts`)
- Continuity chain: `useTeamRuns({ team_name, per_page: 50 })` → client-side prev/next
- Per-iteration breakdown: Group `workspace` entries by `iteration` field. Within each iteration, render Assign (entry_type=assignment), Execute (entry_type=agent_response), Review (entry_type=critic) phases
- Agent sessions linked as: `/sessions/team-${runId}-${agentName}`
- Polling: `refetchInterval: 10000` when status is `running` or `suspended`

#### Phase 4: Cross-Linking Enhancements

**Files to modify:**
- `dashboard/src/pages/TeamRunDetail.tsx` — Links to per-agent sessions, trace detail
- `dashboard/src/pages/SessionDetail.tsx` — Add "originating task" link for callback/team sessions (query: parse session ID pattern or use tasks endpoint with `created_by_session` filter — use `GET /api/v1/tasks?created_by_session=X` which requires adding `created_by_session` to `TaskFilters`)
- `dashboard/src/pages/AgentDetail.tsx` — Add "Recent Tasks" section using `useTasks({ agent_id })`
- `dashboard/src/pages/TraceDetail.tsx` — Ensure task events link to `/tasks/:id` and team_workspace events link to `/team-runs/:runId`

**Cross-link visual language:**
- Trace IDs: monospace, purple `text-purple-400`, `Link` component to `/traces/:id`
- Agent IDs: regular weight, purple, `Link` to `/agents/:id`
- Session IDs: monospace, purple, `Link` to `/sessions/:id`
- Run IDs: monospace, purple, `Link` to `/team-runs/:id`
- Task IDs: monospace, gray `text-gray-500`, purple on hover

## System-Wide Impact

### Interaction Graph

- New `GET /api/v1/tasks` → `dashboard.rs:handle_tasks_list` → `async_db.list_tasks_paginated_with_count` → `db.list_tasks_paginated_with_count` → SQLite `tasks` table
- New `GET /api/v1/team-runs` → `dashboard.rs:handle_team_runs_list` → `async_db.list_team_runs_paginated_with_count` → `db.list_team_runs_paginated_with_count` → SQLite `team_runs` table
- Dashboard auth: Both tokens accepted (MIKA_DASHBOARD_TOKEN, MIKA_INTERNAL_TOKEN) — same as all existing `/api/v1/*` routes

### Error Propagation

- DB errors → `anyhow::Error` → `internal_error()` → 500 JSON → React Query error state → error UI per section
- 404 on task/team-run detail → explicit 404 JSON → React Query error → "Not found" message

### State Lifecycle Risks

- Tasks transition states independently of dashboard reads — stale reads are acceptable (operator debugging, not real-time control)
- No write operations from dashboard — read-only, no risk of partial state

### API Surface Parity

- New endpoints follow identical auth, pagination, and error patterns as existing dashboard endpoints
- `PaginatedResponse<T>` reused for all list endpoints

### Integration Test Scenarios

1. Create a manual work item → verify it appears in `GET /api/v1/tasks?trigger_type=manual`
2. Run a team → verify team run appears in `GET /api/v1/team-runs?status=running` and tasks appear grouped by `team_run_id`
3. Complete a team run → verify status transitions propagate to both endpoints
4. Navigate Tasks page → verify 4 sections load independently with correct filter subsets
5. Click trace_id on task row → verify navigation to Trace Detail shows the task in the unified timeline

## Acceptance Criteria

### Functional Requirements

- [ ] `GET /api/v1/tasks` returns paginated, filtered task list with `TaskResponse` shape
- [ ] `GET /api/v1/team-runs` returns paginated, filtered team run list
- [ ] `GET /api/v1/tasks/:id` returns single task or 404
- [ ] Tasks page renders 4 sections: Work Items, Team Runs, Standalone Callbacks, Scheduled
- [ ] Each section loads independently with correct filters
- [ ] Each section has independent pagination
- [ ] Scheduled section collapsed by default, lazy-loads on expand
- [ ] Team Runs list page with filter bar (team name, status, date range)
- [ ] Team Run detail page with continuity chain, iteration breakdown, workspace table, task tree
- [ ] Team Run detail polls every 10s when status is running/suspended
- [ ] Sidebar shows 6 nav items: Event Timeline, Agents, Sessions, Traces, Tasks, Team Runs
- [ ] Cross-links: team run detail → per-agent sessions
- [ ] Cross-links: task rows → trace detail (via created_trace_id)
- [ ] Cross-links: agent detail → recent tasks section
- [ ] Cross-links: trace detail → task and team_workspace events with links
- [ ] All clickable IDs use consistent visual language (monospace purple for traces/sessions/runs)

### Non-Functional Requirements

- [ ] All Tailwind classes use static mapping objects (no dynamic construction)
- [ ] All React Query error states surfaced to user (no silent failures)
- [ ] Empty states for all sections/pages when no data
- [ ] `TaskResponse` omits `process_id`, truncates `action_config`/`result` to 200 chars
- [ ] DB query methods use `TASK_COLUMNS` constant (no hand-written column lists)
- [ ] New DB methods have roundtrip tests

### Quality Gates

- [ ] `cargo test` passes with new DB tests
- [ ] `cargo clippy` clean
- [ ] `npm run build --prefix dashboard` succeeds
- [ ] Dashboard renders correctly with empty DB (no tasks, no team runs)

## Dependencies & Risks

- **No schema changes required** — all data already exists in tasks and team_runs tables
- **Task struct lacks Serialize** — mitigated by DTO approach (TaskResponse)
- **Risk: Large task tables** — mitigated by pagination (default 50, max 200) and indexed queries (existing partial indexes on tasks table)
- **Risk: SessionDetail overlap** — Team Runs Detail coexists as "operations view" vs SessionDetail as "conversation view"; cross-link between them

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-16-dashboard-tasks-teams-brainstorm.md](../brainstorms/2026-03-16-dashboard-tasks-teams-brainstorm.md) — Key decisions: 4-section Tasks layout, list+detail Team Runs, deep cross-linking over self-contained pages

### Internal References

- Dashboard page pattern: `dashboard/src/pages/Sessions.tsx`
- API hook pattern: `dashboard/src/api/sessions.ts`
- Server handler pattern: `crates/mika-agent/src/server/dashboard.rs`
- DB pagination pattern: `crates/mika-agent/src/db.rs` (search `_with_count`)
- Existing team hooks (unused): `dashboard/src/api/teams.ts`
- Task columns constant: `crates/mika-agent/src/db.rs:1646` (`TASK_COLUMNS`)
- Team run types: `crates/mika-agent/src/db.rs:267` (`TeamRunRow`)

### Learnings Applied

- **Never hand-write SQL column lists** — use `TASK_COLUMNS` constant (from: `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md`)
- **Surface all React Query error states** — never swallow errors silently (from: same)
- **Static Tailwind class mappings** — never construct classes dynamically (from: `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md`)
- **Defensive JSON parsing** — action_config/result may be malformed (from: `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md`)
- **team_workspace agent_id=NULL** in unified_timeline VIEW (from: `docs/solutions/database-issues/trace-id-observability-gaps-callback-team-timeline.md`)
