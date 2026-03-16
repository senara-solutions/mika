# Brainstorm: Dashboard Tasks & Teams Pages

**Date:** 2026-03-16
**Status:** Draft
**Author:** AI-assisted brainstorm
**Builds on:** [2026-03-08-observability-dashboard-brainstorm.md](2026-03-08-observability-dashboard-brainstorm.md)

## What We're Building

Two new top-level dashboard pages — **Tasks** and **Team Runs** — plus deep cross-linking enhancements to the existing four pages (Timeline, Agents, Sessions, Traces). This completes the "Phase 2" vision from the original dashboard brainstorm.

**Audience:** Operator debugging. Every design decision optimizes for diagnosing issues — reading what agents produced, tracing why the critic rejected it, following the delegation chain.

## Why Now

Issues #160, #161, #162 (merged as PRs #167–#169) hardened the trace_id correlation infrastructure:
- `trace_id` column on `team_runs` (schema v10)
- `execution_trace_id` and `parent_session_id` on tasks/sessions (schema v11)
- `team_workspace` in the `unified_timeline` VIEW
- Server `request_id` → agent `trace_id` flow

The data model now supports full cross-subsystem correlation. The dashboard is the missing UI surface.

## Why This Approach

**Stitch-first design:** Design all screens in Stitch before committing to API shapes. The existing `/api/v1/team-runs/:id/summary` endpoint returns rich data; Stitch screens will reveal whether we need to reshape it or add new endpoints.

**Deep cross-linking over self-contained pages:** `trace_id` threads through every subsystem. Clickable IDs everywhere — the dashboard should respect the data model's correlation structure, not hide it behind isolated pages.

## Data Model Reference

### How IDs relate

| ID | Table (PK) | Flows to |
|---|---|---|
| `agent_id` | `agents` | FK in sessions, messages, tasks, audit_events, core_memory |
| `session_id` | `sessions` | FK in messages; naming: `system-{agent_id}`, `team-{run_id}-{agent_name}` |
| `trace_id` | Column (32-char hex) | messages, audit_events, tasks (created + execution), team_runs, team_workspace |
| `run_id` | `team_runs` | FK in team_workspace; tasks via `team_run_id`; sessions as `team-{run_id}-{agent_name}` |
| `task_id` | `tasks` | Self-referential `parent_task_id`; `team_run_id` links to team runs |

### unified_timeline VIEW (4 legs)

1. **messages** → event_type='message', trace_id, session_id, agent_id
2. **audit_events** → event_type='audit', trace_id, session_id, agent_id
3. **tasks** → event_type='task', `COALESCE(execution_trace_id, created_trace_id)`, created_by_session, agent_id
4. **team_workspace** → event_type='team_workspace', trace_id, synthetic `team-{run_id}`, agent_id=NULL

## Key Decisions

### Tasks page: Four-section layout

| Section | What it shows | Entry point for |
|---|---|---|
| **Work Items** | `trigger_type='manual'` tasks as roots. Status badge + linked team run indicator | "What is the agent supposed to be doing?" |
| **Team Runs** | Grouped by `team_run_id`. Team name, run ID, continuity chain link, task tree (invoke_orchestrator parent + resume_agent children with per-agent status) | "What happened in this team execution?" |
| **Standalone Callbacks** | `resume_agent` and `run_skill` tasks NOT attached to a team run. Long-running skill executions | "What background work is happening?" |
| **Scheduled** | Recurring (`cron`) and one-shot tasks. Heartbeat, reflection, etc. | Separate operational concern |

All sections: status filter, sorted by recency, clickable trace_ids and session links.

### Team Runs page: List + detail drill-down

**List page (simple):**
- Columns: team name, run ID (truncated), status badge, start time, iteration count, previous run link (continuity chain)
- Filters: team name, status, date range

**Detail page (where the diagnostic value is):**
- **Header:** Team name, run ID, status, trace_id link, work item link (if applicable)
- **Continuity chain:** Previous run summary inlined, with link to that run's detail page
- **Per-iteration breakdown:** Each assign→execute→review→iterate cycle as a collapsible section
  - Agent assignments with links to their sessions (`team-{run_id}-{agent_name}`)
  - Agent results (preview with expand)
  - Critic verdict (approve/reject + feedback text)
- **Workspace files:** List of `team_workspace` entries per run, with entry_type badges (goal, orchestrator, assignment, critic, deliverable)
- **Task tree:** The invoke_orchestrator parent + resume_agent children, showing delegation flow

### Cross-linking priority (highest first)

1. **Team run detail → per-agent sessions** — highest-value link. Session is where you read what the agent said, tools called, results returned
2. **Task row → trace detail** — any task with `created_trace_id` links to the full cross-subsystem turn view
3. **Session detail → originating task** — reverse link for callback sessions ("what spawned this?")
4. **Agent detail → active/recent tasks** — new section on agent detail page
5. **Trace detail → everything** — already the natural hub via unified_timeline; ensure task and team_workspace events render with appropriate links

### Existing Stitch screen reference

The "Mika Team Collaboration Detail" screen already designed shows:
- Session-centric view with per-agent task cards
- Tool call counts and completion status per agent
- Final Deliverable section (draft badge)

**What needs to change for the Mika version:**
- Add continuity chain (previous run link + summary)
- Per-iteration breakdown (not just final state)
- Workspace file list with provenance
- Critic verdict per iteration (not just final)
- Deep links to sessions, traces, tasks

### New API endpoints needed

| Endpoint | Purpose |
|---|---|
| `GET /api/v1/tasks` | Paginated task list with filters (status, trigger_type, action_type, agent_id, team_run_id) |
| `GET /api/v1/team-runs` | Paginated team run list with filters (team_id, status, date range) |

Existing endpoints already cover detail views:
- `GET /api/v1/team-runs/:id` — metadata
- `GET /api/v1/team-runs/:id/workspace` — workspace entries
- `GET /api/v1/team-runs/:id/summary` — enriched summary

### Sidebar update

Current: Event Timeline, Agents, Sessions, Traces
New: Event Timeline, Agents, Sessions, Traces, **Tasks**, **Team Runs**

## Open Questions

None — all key decisions resolved during brainstorming.

## Stitch Prompt

See the companion Stitch prompt below for generating the new screens in the existing "Mika Observability Dashboard" project.

---

## Stitch Prompt: Extend Mika Observability Dashboard

**Target project:** "Mika Observability Dashboard" (existing, 10 screens)
**Design system:** Dark mode, custom color #7c69f7, Plus Jakarta Sans, round-8, saturation 3, desktop 1280px

Generate 4 new screens for this project. Match the existing visual language exactly — same dark background, same purple accent (#7c69f7), same badge styles, same table patterns, same sidebar navigation seen in the existing screens.

### Screen 1: Tasks Overview Page

A single-page dashboard with 4 collapsible sections, each with its own header and count badge. The sidebar should show "Tasks" as the active nav item (add it below "Traces" in the existing nav).

**Section 1 — Work Items** (top)
- Header: "Work Items" with count badge (e.g., "3 active")
- Table rows showing manual tasks as root items. Columns:
  - Status badge (pending=yellow, in_progress=blue, blocked=orange, completed=green, cancelled=gray)
  - Title/description (truncated to ~80 chars)
  - Agent ID (clickable link, purple text)
  - Source badge (user_request, self_dev)
  - Linked team run indicator — if a team run references this work item, show a small "Team Run →" chip that's clickable
  - Created timestamp (relative, e.g., "2h ago")
- Empty state: "No active work items"

**Section 2 — Team Runs** (second)
- Header: "Team Runs" with count badge
- Each team run is a card/group, not a flat table row. Inside each card:
  - Left: Team name (bold), Run ID (monospace, truncated with copy button), status badge, start time
  - Right: Iteration count (e.g., "Iteration 2/3"), previous run link (chain icon + truncated run ID) if continuity exists
  - Below: Indented task tree showing:
    - Parent: invoke_orchestrator task with status badge
    - Children: resume_agent tasks, each showing agent name, status badge, and a small "→ Session" link
  - Failed or stale children highlighted with a red/orange left border

**Section 3 — Standalone Callbacks** (third)
- Header: "Standalone Callbacks" with count badge
- Flat table: status badge, task type (resume_agent / run_skill), agent name, description, trace_id (clickable monospace link), created timestamp
- Only shows tasks NOT attached to a team_run_id

**Section 4 — Scheduled** (bottom, collapsed by default)
- Header: "Scheduled Tasks" with count badge
- Table: status badge, name/description, schedule (cron expression or "one-shot"), next run time, last run status, agent name

All clickable IDs use the purple accent color. Trace IDs link to /traces/:id. Agent IDs link to /agents/:id. Session links go to /sessions/:id.

### Screen 2: Team Runs List Page

The sidebar should show "Team Runs" as the active nav item (add below "Tasks").

**Filter bar** (top):
- Team name dropdown (e.g., "All Teams", "research-team", "dev-team")
- Status dropdown (running, completed, failed, suspended)
- Date range picker

**Table:**
- Columns: Team Name, Run ID (monospace, truncated), Status badge, Started (relative time), Iterations (e.g., "2/3"), Previous Run (chain icon + ID or "—"), Trace ID (clickable)
- Row click navigates to Team Run Detail
- Alternating row shading matching the existing timeline table style
- Pagination controls at bottom

### Screen 3: Team Run Detail Page

Reached by clicking a row on the Team Runs List. This is the most information-dense screen — where real debugging happens.

**Header section:**
- Breadcrumb: "Team Runs > research-team > run-abc123"
- Team name (large), Run ID (monospace with copy), Status badge (large), Trace ID (clickable link)
- If linked to a work item: "Work Item: [title]" link

**Continuity Chain panel** (collapsible, below header):
- If this run has a previous_run_id: Show a card with the previous run's summary (2-3 lines), status, iteration count, and a "View Previous Run →" link
- If no previous run: "First run in chain"

**Per-Iteration Timeline** (main content area):
Each iteration (1, 2, 3...) is a collapsible card with a left-side step indicator (vertical line with numbered circles, like a stepper).

Inside each iteration card:
- **Phase: Assign** — Orchestrator assignments text block. List of agent names with their assigned tasks.
- **Phase: Execute** — Per-agent results. Each agent shown as a sub-card:
  - Agent avatar/icon, name (clickable → /agents/:id), status badge
  - Result preview (first 200 chars, expandable)
  - "View Session →" link (goes to /sessions/team-{runId}-{agentName})
  - Tool call count badge
- **Phase: Review** — Critic verdict section:
  - Verdict badge: APPROVED (green) or REJECTED (red/orange)
  - Critic feedback text (full, not truncated — this is diagnostic gold)
  - If rejected: highlighted box showing what needs to change

**Workspace Files** (below iterations):
- Table of team_workspace entries for this run
- Columns: Entry Type badge (goal, orchestrator, assignment, critic, deliverable — each a different color), Content preview (truncated), Agent (if applicable), Timestamp, Trace ID link

**Task Tree** (bottom, collapsible):
- Visual tree: invoke_orchestrator parent → resume_agent children
- Each node: task ID (monospace), status badge, agent name, created time, execution trace link
- Failed tasks highlighted

### Screen 4: Enhanced Sidebar Navigation

Show the full sidebar in isolation to demonstrate the updated navigation with 6 items:
1. Event Timeline (existing)
2. Agents (existing)
3. Sessions (existing)
4. Traces (existing)
5. Tasks (new — use a checklist/checkbox icon)
6. Team Runs (new — use a users/group icon)

The sidebar should match the existing dark sidebar style with the purple active indicator seen in the current screens.

### Cross-linking visual language

Throughout all screens, maintain consistent link styling:
- **Trace IDs:** Monospace, purple (#7c69f7), with a small external-link icon on hover
- **Agent IDs:** Regular weight, purple, with agent avatar/icon inline
- **Session IDs:** Monospace, purple, with a chat-bubble icon
- **Run IDs:** Monospace, purple, with a chain-link icon
- **Task IDs:** Monospace, dimmed gray unless hovered, then purple

### Data to use in mockups

Use realistic sample data:
- Team names: "research-team", "content-team"
- Agent names: "mika" (default), "researcher", "writer", "reviewer"
- Task descriptions: "Analyze Q1 metrics report", "Draft blog post on AI safety", "Review compliance checklist"
- Work item titles: "Prepare weekly briefing", "Investigate login latency spike"
- Critic feedback: "The researcher's analysis lacks quantitative data. The metrics report references 'significant improvement' without specific numbers. Requesting iteration with concrete figures."
