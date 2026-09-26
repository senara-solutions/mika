---
title: "feat: Observability Dashboard"
type: feat
status: active
date: 2026-03-08
origin: docs/brainstorms/2026-03-08-observability-dashboard-brainstorm.md
---

# feat: Observability Dashboard

## Overview

A web-based observability dashboard for Mika, providing visibility into agent runtime behavior — conversations, memory mutations, task scheduling, and cross-subsystem event correlation via the `unified_timeline` VIEW. Standalone React app at `dashboard/` communicating with `mika-spirit` via REST API.

**MVP scope:** 3 views — Unified Timeline (home), Agents, Sessions.

## Problem Statement / Motivation

The orthogonal observability work (PR #88) introduced `trace_id` correlation and the `unified_timeline` VIEW, creating a data foundation for cross-subsystem debugging. Without a visual dashboard, this data is only accessible via raw SQL queries against the SQLite database. Operators need a way to:

- See what the agent is doing in real-time (timeline of events across messages, audit log, tasks)
- Inspect agent state (core memory, sessions, conversation history)
- Correlate events across subsystems by clicking a trace_id

## Proposed Solution

### Frontend: React Dashboard (`dashboard/`)

Standalone Vite + React 19 + TypeScript + Tailwind CSS v4 app, matching the existing `site/` design system (see brainstorm: design system tokens). Uses React Router for navigation and TanStack Query for data fetching with 5s polling.

### Backend: REST API on mika-spirit (`/api/v1/*`)

~10 new read-only GET endpoints on `mika-spirit`, sharing the existing Bearer token auth (`MIKA_INTERNAL_TOKEN`). Requires:
- **Unscoped `AsyncDatabase` handle** — new constructor with no `agent_id` scoping for cross-agent queries
- **CORS middleware** — `tower-http` CorsLayer with configurable origin
- **New query methods** on `Database` for paginated, unscoped data access

### API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/timeline` | Paginated unified timeline, filterable by agent_id, event_type, trace_id, session_id, date range |
| `GET` | `/api/v1/timeline/trace/:trace_id` | All events for a specific trace_id |
| `GET` | `/api/v1/agents` | List all agents with status, last_seen, message count |
| `GET` | `/api/v1/agents/:id` | Agent detail: core memory blocks, soul.md content |
| `GET` | `/api/v1/agents/:id/sessions` | Paginated sessions for an agent |
| `GET` | `/api/v1/agents/:id/audit` | Paginated audit events for an agent |
| `GET` | `/api/v1/sessions` | Paginated sessions, filterable by agent_id, channel_type |
| `GET` | `/api/v1/sessions/:id` | Session detail with metadata |
| `GET` | `/api/v1/sessions/:id/messages` | Paginated messages for a session |

### Pagination Contract

```rust
#[derive(Serialize, ToSchema)]
struct PaginatedResponse<T> {
    data: Vec<T>,
    total: u64,
    page: u32,      // 1-indexed
    per_page: u32,   // default 50, max 200
}
```

- Pages are 1-indexed. Out-of-range pages return empty `data` array (not 404).
- `total` computed via `COUNT(*)` subquery.
- Default sort: `created_at DESC` (newest first).
- Query params: `?page=1&per_page=50` with `#[serde(default)]`.

### Timestamp Contract

- API returns Unix timestamps (integers, matching DB storage).
- Filter params `from` and `to` accept Unix timestamps.
- Frontend converts to user's local timezone for display.

### Image Handling

Base64 image data in `tool_result` messages is stripped in the API response. Replaced with a placeholder: `[image: {mime_type}, {size}]`. Prevents multi-MB payloads on message list endpoints.

### CORS Configuration

New env var `MIKA_CORS_ORIGIN` (default: `http://localhost:5173`). Applied via `tower-http::cors::CorsLayer` in `build_router()`.

### Auth: Token Delivery to SPA

Build-time `VITE_MIKA_TOKEN` env var baked into the JS bundle. Acceptable for Phase 1 (localhost-only). The dashboard sends it as `Authorization: Bearer <token>` on every API request.

### Auto-Refresh Behavior

Smart refresh via TanStack Query `refetchInterval`:
- **Active on page 1 with default filters** — polls every 5s, uses `keepPreviousData` to prevent layout shift.
- **Paused when user is on page >1 or has active filters** — shows a "New events available" indicator. User clicks to refresh.
- **Paused on detail views** (trace detail, agent detail, session detail) — no background polling.

## Technical Considerations

### Unscoped AsyncDatabase (new pattern)

The existing `AsyncDatabase` scopes all queries to an `agent_id`. Dashboard endpoints need cross-agent access.

**Approach:** Add `AsyncDatabase::new_unscoped(db: Arc<Mutex<Database>>)` that stores `agent_id: None`. Add new query methods to `Database` that don't take `agent_id`:

```rust
// db.rs — new methods
pub fn query_timeline(&self, filters: &TimelineFilters, limit: u32, offset: u32) -> Result<Vec<TimelineRow>>
pub fn query_timeline_count(&self, filters: &TimelineFilters) -> Result<u64>
pub fn list_all_agents(&self) -> Result<Vec<AgentRow>>
pub fn list_all_sessions(&self, filters: &SessionFilters, limit: u32, offset: u32) -> Result<Vec<SessionListRow>>
pub fn get_session_messages(&self, session_id: &str, limit: u32, offset: u32) -> Result<Vec<SessionMessage>>
```

The unscoped handle is stored in `AppState` alongside the per-agent handles:

```rust
// state.rs
pub struct AppState {
    pub agents: Arc<HashMap<String, Arc<AgentState>>>,
    pub dashboard_db: AsyncDatabase,  // unscoped, for dashboard endpoints
    // ... existing fields
}
```

**Key files:**
- `crates/mika-agent/src/async_db.rs` — add `new_unscoped()` constructor
- `crates/mika-agent/src/db.rs` — add unscoped query methods
- `crates/mika-agent/src/server/state.rs` — add `dashboard_db` field

### Unified Timeline Query Performance

The `unified_timeline` VIEW is a `UNION ALL` across 3 tables. With filters and `ORDER BY ... LIMIT ... OFFSET`, SQLite must materialize the union before sorting. For the MVP with moderate data volumes (< 50K rows total), this is acceptable. Monitor and optimize if needed.

**Mitigation:** If slow, replace the VIEW query with 3 separate queries merged and sorted in Rust.

### soul.md File Access

Agent detail endpoint reads `{agent_home}/soul.md` via `tokio::fs::read_to_string`. Returns empty string if file doesn't exist. The `AgentState.home_dir` path is already available.

### CORS: tower-http Feature

Add `"cors"` to tower-http features in workspace `Cargo.toml`:

```toml
tower-http = { version = "0.6", features = ["trace", "limit", "set-header", "cors"] }
```

## System-Wide Impact

- **Interaction graph:** Dashboard endpoints are read-only GET handlers. They do not interact with the agent loop, task engine, or message sender. No callbacks, no middleware side effects beyond auth and tracing.
- **Error propagation:** Database query errors → 500 JSON response. Auth failures → 401. Bad query params → 400. No retry logic needed (client-side polling handles transient failures).
- **State lifecycle risks:** None — read-only endpoints cannot create inconsistent state.
- **API surface parity:** The new `/api/v1/*` endpoints are dashboard-specific. Existing `/message` and `/tasks/{id}/complete` endpoints are unchanged. Both share the same auth token (acknowledged security note: dashboard token grants write access to other endpoints).
- **Integration test scenarios:**
  1. Dashboard endpoint returns data after agent processes a message (write via `/message`, read via `/api/v1/timeline`)
  2. CORS preflight succeeds from dashboard origin, fails from unknown origin
  3. Pagination returns correct total count and page boundaries
  4. Timeline filtering by trace_id returns events from all 3 subsystems
  5. Auth failure returns 401 JSON (not HTML)

## Acceptance Criteria

### Backend (mika-spirit)

- [x] `GET /api/v1/timeline` returns paginated unified timeline, filterable by agent_id, event_type, trace_id, session_id, from/to date range
- [x] `GET /api/v1/timeline/trace/:trace_id` returns all events for a trace_id across messages, audit_events, and tasks
- [x] `GET /api/v1/agents` returns all agents with active status, last_seen, and message count
- [x] `GET /api/v1/agents/:id` returns agent detail with core memory blocks and soul.md content
- [x] `GET /api/v1/agents/:id/sessions` and `/agents/:id/audit` return paginated, agent-scoped data
- [x] `GET /api/v1/sessions` returns paginated sessions, filterable by agent_id and channel_type
- [x] `GET /api/v1/sessions/:id/messages` returns paginated messages with base64 images stripped
- [x] All endpoints use Bearer auth (MIKA_INTERNAL_TOKEN) and return JSON errors
- [x] CORS configured via `MIKA_CORS_ORIGIN` env var
- [ ] OpenAPI spec updated with all new endpoints
- [ ] Handler tests for each endpoint using existing `test_state()` pattern

### Frontend (dashboard/)

- [x] Vite + React 19 + TypeScript + Tailwind v4 project matching site/ design system
- [x] React Router with routes: `/` (timeline), `/traces/:id`, `/agents`, `/agents/:id`, `/sessions`, `/sessions/:id`
- [x] TanStack Query for all data fetching with 5s smart polling on timeline
- [x] Timeline view: paginated table with filter bar (agent, event_type, date range, trace_id search)
- [x] Trace detail view: all events for a trace_id, grouped chronologically
- [x] Agents view: card grid with agent stats, click to see core memory + sessions + audit tabs
- [x] Sessions view: filterable list, click to see conversation with role-based message rendering
- [x] Filter state persisted in URL query parameters (deep linking, back button support)
- [x] Loading states (skeleton/spinner), empty states, error states for all views
- [x] Bearer token from `VITE_MIKA_TOKEN` env var sent on every request

## Success Metrics

- Dashboard loads and displays data from a running mika-spirit within 2 seconds
- Timeline view correctly correlates events across all 3 subsystems by trace_id
- All list views paginate correctly with accurate total counts
- Smart auto-refresh does not disrupt user context (no scroll reset, no filter loss)

## Dependencies & Risks

| Dependency/Risk | Mitigation |
|----------------|------------|
| `unified_timeline` VIEW performance at scale | Monitor query times. Fallback: query 3 tables separately in Rust. |
| Shared auth token grants write access | Acceptable for Phase 1 (localhost). Document. Consider read-only token in Phase 2. |
| CORS misconfiguration blocks all dashboard requests | Test CORS preflight in CI. Configurable origin via env var. |
| Build-time token in JS bundle | Phase 1 is localhost-only. Phase 2: login form or proxy. |
| soul.md filesystem reads from async handler | Use `tokio::fs::read_to_string` with graceful fallback to empty string. |
| New `AsyncDatabase` unscoped constructor | Follows established pattern. Risk: forgetting to scope agent-specific queries. Mitigate with clear method naming (`list_all_*` vs `list_*`). |

## Project Structure

```
dashboard/
  package.json
  vite.config.ts
  tsconfig.json
  index.html
  src/
    main.tsx
    App.tsx                    # Router setup
    index.css                  # Tailwind v4 theme tokens (from site/)
    api/
      client.ts                # Fetch wrapper with Bearer auth
      timeline.ts              # TanStack Query hooks for timeline
      agents.ts                # TanStack Query hooks for agents
      sessions.ts              # TanStack Query hooks for sessions
    components/
      Layout.tsx               # Sidebar + main content
      Sidebar.tsx              # Navigation sidebar
      DataTable.tsx            # Reusable paginated table
      Pagination.tsx           # Page controls
      FilterBar.tsx            # Filter inputs
      StatusBadge.tsx          # Agent/task status indicator
      TraceLink.tsx            # Clickable trace_id
      ChatMessage.tsx          # Message bubble (role-aware)
      EmptyState.tsx           # Empty state placeholder
      NewEventsIndicator.tsx   # "New events available" banner
    pages/
      Timeline.tsx
      TraceDetail.tsx
      Agents.tsx
      AgentDetail.tsx
      Sessions.tsx
      SessionDetail.tsx
```

## Sources & References

- **Origin brainstorm:** [docs/brainstorms/2026-03-08-observability-dashboard-brainstorm.md](../brainstorms/2026-03-08-observability-dashboard-brainstorm.md) — key decisions: standalone app, REST API, Bearer auth, polling, read-only MVP, React Router + TanStack Query
- **Server patterns:** `crates/mika-agent/src/server/mod.rs` (router), `handlers.rs` (handler signatures), `types.rs` (serde + utoipa), `auth.rs` (Bearer middleware)
- **Database schema:** `crates/mika-agent/src/db.rs` (unified_timeline VIEW at line ~25, all query methods)
- **AsyncDatabase:** `crates/mika-agent/src/async_db.rs` (agent-scoped pattern, `with_db` closure dispatch)
- **Design system:** `site/src/index.css` (theme tokens), `site/src/components/` (component patterns)
- **ADR-001:** `docs/adr/001-axum-http-server-architecture.md` (middleware ordering, auth pattern)
- **Trace correlation:** `docs/solutions/architecture-patterns/trace-id-correlation-unified-observability.md`
