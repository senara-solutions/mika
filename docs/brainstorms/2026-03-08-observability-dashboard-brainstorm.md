# Brainstorm: Mika Observability Dashboard

**Date:** 2026-03-08
**Status:** Draft
**Author:** AI-assisted brainstorm

## What We're Building

A web-based observability and management dashboard for Mika. It provides visibility into the agent's runtime behavior — conversations, memory mutations, task scheduling, and cross-subsystem event correlation via the `unified_timeline` VIEW introduced in the orthogonal observability work.

**MVP scope (Phase 1):** 3 views — Unified Timeline (home), Agents, Sessions.

**Future views (Phase 2+):** Tasks, Teams, Memory, Skills.

The dashboard is a standalone React app at `dashboard/` in the repo, communicating with `mika-spirit` via REST API endpoints.

## Why This Approach

The orthogonal observability work (trace_id correlation, unified_timeline VIEW) created the data foundation. Without a dashboard, this data is only accessible via raw SQL queries. A visual dashboard makes the observability system actually usable for debugging, monitoring, and understanding agent behavior.

### Why standalone `dashboard/` (not inside `site/`)
- The marketing site (`site/`) and ops dashboard serve completely different audiences and purposes
- Clean separation prevents coupling a public site with internal tooling
- Same Vite + React + Tailwind v4 stack, same design tokens — just a separate app

### Why REST over single-query endpoint
- Standard `/api/v1/*` routes are discoverable, cacheable, and easy to evolve
- Each view maps cleanly to 2-3 endpoints
- OpenAPI spec can be extended naturally

### Why polling over SSE/WebSocket
- mika-spirit is a simple Axum app — adding SSE infrastructure is premature
- 5-second polling is fine for an ops dashboard (not a real-time chat UI)
- Can upgrade to SSE later without changing the dashboard architecture

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| App location | `dashboard/` at repo root | Sibling to `site/`, same standalone pattern |
| API style | REST under `/api/v1/*` | Conventional, discoverable, cacheable |
| Auth | Bearer token (MIKA_INTERNAL_TOKEN) | Reuse existing auth mechanism |
| Real-time | Polling every 5s | Simple, sufficient for ops tooling |
| Read/Write | Read-only Phase 1 | Pure observability, no mutation risk |
| MVP scope | Timeline + Agents + Sessions | Most valuable views first |
| Framework | React 19 + TypeScript + Vite + Tailwind v4 | Matches existing site/ stack |
| Routing | React Router | Standard, well-supported |
| Data fetching | TanStack Query | Excellent polling, pagination, caching support |
| Design | Match site/ exactly | Same dark theme, colors, typography, components |

## Design System (from site/)

Extracted from `site/src/index.css`:

```
Background:      #0d0f12 (--color-bg)
Card background: #151820 (--color-bg-card)
Accent purple:   #7c6af7 (--color-accent)
Accent hover:    #9d8fff (--color-accent-light)
Heading text:    #e8ecf2 (--color-heading)
Muted text:      #a0a8b8 (--color-muted)
Font sans:       "Plus Jakarta Sans" (300-800)
Font mono:       "JetBrains Mono" (400/500/600)
Card radius:     rounded-2xl
Card border:     border-white/[0.05]
Card hover:      hover:border-accent/40
```

Status colors: `emerald-400` (success), `amber-400` (warning), `red-400` (error).

## MVP Views

### 1. Unified Timeline (Home — `/`)

The primary view. Shows the `unified_timeline` VIEW data.

**API endpoints:**
- `GET /api/v1/timeline?agent_id=&event_type=&trace_id=&session_id=&from=&to=&page=&per_page=`
- `GET /api/v1/timeline/trace/:trace_id` — all events for a specific trace

**UI:**
- Filter bar at top: agent selector, event type dropdown, date range picker, trace_id search
- Paginated table: timestamp, agent, event_type, event_subtype, summary (truncated)
- Click a row's trace_id to expand/navigate to trace detail view
- Trace detail: all events for that trace_id grouped chronologically across messages, audit, tasks
- Auto-refresh every 5s (TanStack Query `refetchInterval`)

### 2. Agents (`/agents`)

**API endpoints:**
- `GET /api/v1/agents` — list all agents with status, last_seen, message count
- `GET /api/v1/agents/:id` — agent detail (core memory blocks, soul.md content)
- `GET /api/v1/agents/:id/sessions?page=&per_page=` — recent sessions for agent
- `GET /api/v1/agents/:id/audit?page=&per_page=` — recent audit events for agent

**UI:**
- Card grid showing each agent: name, active status, last_seen, message count
- Click agent card to see detail view with tabs: Overview (core memory), Sessions, Audit Log
- Core memory shown as labeled text blocks (self_model, user_summary, current_priorities, key_people)
- soul.md rendered as markdown

### 3. Sessions (`/sessions`)

**API endpoints:**
- `GET /api/v1/sessions?agent_id=&channel_type=&page=&per_page=` — list recent sessions
- `GET /api/v1/sessions/:id/messages?page=&per_page=` — messages for a session

**UI:**
- Filterable list: agent, channel_type (cli/telegram/team/system)
- Each row: session id (truncated), agent, channel_type, started_at, message count
- Click to see full conversation — messages rendered as chat bubbles (user/assistant)
- tool_result and system messages shown differently (muted, collapsible)

## API Implementation Notes

All new endpoints go in `crates/mika-agent/src/server/`. They share the existing `AppState` which holds the `AsyncDatabase`.

**Response pattern:**
```rust
#[derive(Serialize)]
struct PaginatedResponse<T> {
    data: Vec<T>,
    total: u64,
    page: u32,
    per_page: u32,
}
```

**CORS:** Dashboard runs on a different port than mika-spirit. Need `tower-http` CORS middleware (already a dependency).

**Query parameters:** Use `axum::extract::Query<T>` with `#[serde(default)]` for optional filters.

## Project Structure

```
dashboard/
  package.json
  vite.config.ts
  tsconfig.json
  index.html
  src/
    main.tsx
    App.tsx
    index.css              # Theme tokens (copied from site/)
    api/
      client.ts            # Base fetch wrapper with auth header
      timeline.ts          # Timeline API hooks
      agents.ts            # Agents API hooks
      sessions.ts          # Sessions API hooks
    components/
      Layout.tsx            # Sidebar nav + main content area
      Sidebar.tsx
      FilterBar.tsx
      DataTable.tsx         # Reusable paginated table
      Pagination.tsx
      StatusBadge.tsx
      TraceLink.tsx
      ChatMessage.tsx       # Message bubble for session detail
    pages/
      Timeline.tsx
      TraceDetail.tsx
      Agents.tsx
      AgentDetail.tsx
      Sessions.tsx
      SessionDetail.tsx
    hooks/
      useFadeIn.ts          # From site/
```

## Phase 2 Views (Future)

Deferred to keep Phase 1 focused:

- **Tasks:** List/filter tasks, parent-child tree view, overdue highlighting
- **Teams:** Team runs, workspace entries, phase timeline
- **Memory:** Tabbed view (core memory, people, commitments, preferences, events, audit log)
- **Skills:** Installed skills per agent, recent skill invocations

## Resolved Questions

- **App location:** `dashboard/` at repo root (standalone, sibling to `site/`)
- **API style:** REST under `/api/v1/*`
- **Auth:** Bearer token using existing MIKA_INTERNAL_TOKEN
- **Real-time:** 5s polling via TanStack Query
- **Mutations:** Read-only for Phase 1
- **MVP scope:** Timeline + Agents + Sessions (3 views)
- **Libraries:** React Router + TanStack Query

## Open Questions

_(None — all questions resolved during brainstorm)_
