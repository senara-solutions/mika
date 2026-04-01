---
title: "Dashboard: add RESTful detail pages for Tasks, LLM Calls, Tool Calls and fix Dev Run detail"
type: feat
status: active
date: 2026-04-01
issue: 361
---

# Dashboard: Add RESTful Detail Pages for Tasks, LLM Calls, Tool Calls and Fix Dev Run Detail

## Overview

The observability dashboard is missing RESTful detail pages for Tasks, LLM Calls, and Tool Calls — these entities only have list views with no `/:id` detail routes. Additionally, the Dev Run detail page has bugs (plain text ID, broken PR field, unnecessary merge button). This plan adds three new detail pages, two new backend endpoints, fixes Dev Run detail, and removes the merge feature.

## Problem Statement / Motivation

Every other dashboard entity (sessions, agents, traces, team runs, dev runs) has a detail page. Tasks, LLM Calls, and Tool Calls are missing detail pages, creating gaps in the investigation workflow. Users cannot deep-dive into individual records from list views. The Dev Run detail page has three bugs visible in production.

## Proposed Solution

### Phase 1: Backend — DB Layer + API Endpoints

**1a. DB layer (`db.rs` + `async_db.rs`)**

Add single-record getters following the `get_task_unscoped` pattern:

- `get_llm_call_by_id(&self, id: i64) -> Result<Option<LlmCallRow>>` — uses `row_to_llm_call` mapper
- `get_tool_call_by_id(&self, id: i64) -> Result<Option<ToolCallRow>>` — uses `row_to_tool_call` mapper
- Async wrappers in `async_db.rs`

Note: `get_task_unscoped` already exists for tasks. However, the existing `TaskResponse` DTO truncates `action_config` and `result` to 200 chars. For the detail page, create a `TaskDetailResponse` that returns full fields (or add a `full: bool` parameter to the existing response conversion).

**Pattern reference:**
```rust
// db.rs — follows get_task_unscoped pattern
pub fn get_llm_call_by_id(&self, id: i64) -> Result<Option<LlmCallRow>> {
    self.conn
        .query_row(
            "SELECT id, trace_id, session_id, agent_id, model, provider, ... FROM llm_calls WHERE id = ?1",
            params![id],
            Self::row_to_llm_call,
        )
        .optional()
        .map_err(Into::into)
}
```

**1b. API handlers (`dashboard.rs`)**

- `handle_llm_call_detail(State, Path<i64>)` — returns `LlmCallRow` or 404
- `handle_tool_call_detail(State, Path<i64>)` — returns `ToolCallRow` or 404
- `handle_task_detail_full(State, Path<String>)` — returns `TaskDetailResponse` with untruncated fields (or modify existing handler to return full fields)

Follow the `handle_task_detail` pattern with `Ok(Some(x)) / Ok(None) / Err(e)` match arms.

**1c. Routes (`mod.rs`)**

Add:
- `GET /api/v1/llm-calls/{id}` → `handle_llm_call_detail`
- `GET /api/v1/tool-calls/{id}` → `handle_tool_call_detail`

Remove:
- `POST /api/v1/dev-runs/{task_id}/merge` → `handle_dev_run_merge`

**1d. Remove merge endpoint (`dashboard_dev_runs.rs`)**

- Delete `handle_dev_run_merge` handler function
- Delete `MergeResponse` struct
- Remove route registration in `mod.rs`

**1e. OpenAPI (`openapi.rs`)**

- Add `#[utoipa::path]` annotations to new handlers
- Register in `AgentApiDoc` paths
- Remove merge endpoint from OpenAPI
- Regenerate spec: `cargo test -p mika-agent --lib -- write_agent_openapi_yaml --ignored`

### Phase 2: Frontend — API Hooks

**2a. New hooks**

`dashboard/src/api/llmCalls.ts`:
```typescript
export function useLlmCall(id: string | undefined) {
  return useQuery<LlmCallRow>({
    queryKey: ['llm-call', id],
    queryFn: () => apiFetch(`/llm-calls/${id}`),
    enabled: !!id,
  })
}
```

`dashboard/src/api/toolCalls.ts`:
```typescript
export function useToolCall(id: string | undefined) {
  return useQuery<ToolCallRow>({
    queryKey: ['tool-call', id],
    queryFn: () => apiFetch(`/tool-calls/${id}`),
    enabled: !!id,
  })
}
```

**2b. Remove `useMergeDevRun`**

`dashboard/src/api/devRuns.ts`: Remove `useMergeDevRun` mutation hook and its types.

### Phase 3: Frontend — Detail Pages

**3a. Extract `MetadataRow` component**

Create `dashboard/src/components/MetadataRow.tsx` — extract from `DevRunDetail.tsx` to avoid duplication across 4 detail pages:
```tsx
export function MetadataRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3 py-2">
      <span className="text-muted/60 text-xs w-28 shrink-0 uppercase tracking-wider">{label}</span>
      <span className="text-heading text-sm">{children}</span>
    </div>
  )
}
```

**3b. `TaskDetail.tsx`** (`dashboard/src/pages/TaskDetail.tsx`)

- Uses `useTask(taskId)` (existing hook) + `useTaskChildren(taskId)` (existing)
- Early-return pattern: loading → error → not found → content
- Back link: `← Back to Tasks`
- Metadata card: ID (with CopyButton), label, status (TaskStatusBadge), trigger type, action type, source, agent (link to `/agents/:id`), session (link to `/sessions/:id`), trace (link to `/traces/:id`), parent task (link to `/tasks/:id`), team run (link to `/team-runs/:id`), reference URL (external link), timestamps (created, scheduled, completed)
- Full action_config and result sections with `<pre>` blocks and CopyButton
- Child tasks section (if any exist) with links to child task detail pages

**3c. `LlmCallDetail.tsx`** (`dashboard/src/pages/LlmCallDetail.tsx`)

- Uses new `useLlmCall(id)` hook
- Early-return pattern
- Back link: `← Back to LLM Calls`
- Metadata card: ID (CopyButton), model, provider, step number, tokens (input/output/cache read/cache write), stop reason, status, latency, error message (if any), agent (link), session (link), trace (link), timestamps

**3d. `ToolCallDetail.tsx`** (`dashboard/src/pages/ToolCallDetail.tsx`)

- Uses new `useToolCall(id)` hook
- Early-return pattern
- Back link: `← Back to Tool Calls`
- Metadata card: ID (CopyButton), tool name, source badge, skill name (if any), success/error status, latency, step, agent (link), session (link), trace (link), LLM call (link to `/llm-calls/:id`)
- Full input and output sections with `<pre>` blocks and CopyButton (these can be up to 50KB each — the whole point of the detail page)

### Phase 4: Frontend — Router + List Page Updates

**4a. Router (`App.tsx`)**

Add three new routes:
```tsx
<Route path="tasks/:taskId" element={<TaskDetail />} />
<Route path="llm-calls/:id" element={<LlmCallDetail />} />
<Route path="tool-calls/:id" element={<ToolCallDetail />} />
```

**4b. Tasks list (`Tasks.tsx`)**

Make task labels/IDs clickable links to `/tasks/:taskId`. Use `e.stopPropagation()` on the link to avoid conflicting with `ExpandableTaskRow` expand/collapse behavior. Follow the existing trace_id link pattern:
```tsx
<Link to={`/tasks/${task.id}`} className="text-accent text-xs font-mono hover:text-accent-light transition-colors" onClick={e => e.stopPropagation()}>
  {task.label || task.id.slice(0, 8) + '...'}
</Link>
```

**4c. LLM Calls list (`LlmCalls.tsx`)**

Add clickable link on the model or a dedicated ID column linking to `/llm-calls/:id`.

**4d. Tool Calls list (`ToolCalls.tsx`)**

Add clickable link on tool name or a dedicated link element. Use `e.stopPropagation()` to coexist with the existing row-level expand/collapse for inline input/output preview.

**4e. Cross-linking from TraceDetail and SessionDetail**

Update the LLM call and tool call sub-tables in `TraceDetail.tsx` and `SessionDetail.tsx` to link to the new detail pages. These tables already show per-trace and per-session LLM/tool calls — making the rows or IDs link to `/llm-calls/:id` and `/tool-calls/:id` provides consistent navigation.

### Phase 5: DevRunDetail Fixes

**5a. ID → Link to task detail**

Replace plain text `{run.id}` with:
```tsx
<Link to={`/tasks/${run.id}`} className="text-accent text-xs font-mono hover:text-accent-light transition-colors">
  {run.id}
</Link>
```

**5b. PR field null safety**

Change condition from `run.pr_url` to `run.pr_url && run.pr_number`:
```tsx
{run.pr_url && run.pr_number ? (
  <a href={run.pr_url} ...>#{run.pr_number}</a>
) : run.pr_url ? (
  <a href={run.pr_url} ...>{run.pr_url}</a>
) : '—'}
```

**5c. Remove merge button**

- Remove `useMergeDevRun` import and `mergeMutation` usage
- Remove `showConfirm` state
- Remove `canMerge` logic
- Remove merge button UI (confirm flow, button, success/error alerts)

## Technical Considerations

### Architecture

- All changes follow existing patterns — no new architectural concepts
- Detail pages reuse existing row mapper functions in the DB layer
- Frontend follows the established DevRunDetail early-return pattern

### Key Conventions (from learnings)

1. **UTF-8 safety**: Use `truncate_chars()` or `floor_char_boundary()` for any string truncation — never byte-slice ([docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md](docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md))
2. **Column constants**: Use consistent SELECT column lists — positional `row.get(N)` breaks silently when columns change ([docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md](docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md))
3. **OpenAPI sync**: Add `#[utoipa::path]` annotations to ALL new handlers, register in `AgentApiDoc`, regenerate spec ([docs/solutions/integration-issues/openapi-spec-drift-missing-utoipa-annotations.md](docs/solutions/integration-issues/openapi-spec-drift-missing-utoipa-annotations.md))
4. **Tailwind classes**: Never construct classes dynamically — use explicit class names ([docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md](docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md))
5. **Shared UI imports**: Always import from `@senara-solutions/ui` barrel export ([docs/solutions/architecture-patterns/extract-shared-ui-package.md](docs/solutions/architecture-patterns/extract-shared-ui-package.md))

### Performance

- Single-record queries by primary key (`WHERE id = ?1`) are O(1) index lookups — negligible overhead
- Tool call input/output can be up to 50KB each — detail pages should use `<pre>` with `overflow-auto` and `max-height` to prevent layout blowout

### Security

- New endpoints use existing dashboard auth middleware (dashboard token or internal token)
- No new auth surface — follows existing pattern

## Acceptance Criteria

- [ ] `GET /api/v1/llm-calls/{id}` returns single LLM call record or 404
- [ ] `GET /api/v1/tool-calls/{id}` returns single tool call record or 404
- [ ] `POST /api/v1/dev-runs/{task_id}/merge` endpoint removed
- [ ] New endpoints have `#[utoipa::path]` annotations and are registered in `AgentApiDoc`
- [ ] OpenAPI spec regenerated and synced
- [ ] `/tasks/:taskId` detail page shows full task metadata with cross-links
- [ ] `/llm-calls/:id` detail page shows full LLM call details with cross-links
- [ ] `/tool-calls/:id` detail page shows full tool call details (including full input/output) with cross-links
- [ ] Task IDs in task list are clickable links to task detail
- [ ] LLM call rows in list have clickable links to LLM call detail
- [ ] Tool call rows in list have clickable links to tool call detail (coexists with expand/collapse)
- [ ] TraceDetail and SessionDetail sub-tables link to new detail pages
- [ ] DevRunDetail ID is a link to `/tasks/:id`
- [ ] DevRunDetail PR field shows `—` when `pr_number` is null (not `#`)
- [ ] DevRunDetail merge button, confirmation flow, and success/error alerts removed
- [ ] `useMergeDevRun` hook removed from `devRuns.ts`
- [ ] `MetadataRow` component extracted to shared location
- [ ] All detail pages follow early-return loading/error/not-found pattern
- [ ] `cargo test` passes
- [ ] `cargo clippy` passes
- [ ] Dashboard builds successfully (`npm run build --prefix dashboard`)

## Files to Modify

### Backend (Rust)

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` | Add `get_llm_call_by_id`, `get_tool_call_by_id` |
| `crates/mika-agent/src/async_db.rs` | Add async wrappers |
| `crates/mika-agent/src/server/dashboard.rs` | Add `handle_llm_call_detail`, `handle_tool_call_detail` |
| `crates/mika-agent/src/server/dashboard_dev_runs.rs` | Remove `handle_dev_run_merge`, `MergeResponse` |
| `crates/mika-agent/src/server/mod.rs` | Add detail routes, remove merge route |
| `crates/mika-agent/src/server/openapi.rs` | Add/remove utoipa paths |

### Frontend (React/TypeScript)

| File | Change |
|------|--------|
| `dashboard/src/App.tsx` | Add 3 new routes |
| `dashboard/src/components/MetadataRow.tsx` | New shared component |
| `dashboard/src/pages/TaskDetail.tsx` | New detail page |
| `dashboard/src/pages/LlmCallDetail.tsx` | New detail page |
| `dashboard/src/pages/ToolCallDetail.tsx` | New detail page |
| `dashboard/src/pages/Tasks.tsx` | Make IDs clickable |
| `dashboard/src/pages/LlmCalls.tsx` | Make rows clickable |
| `dashboard/src/pages/ToolCalls.tsx` | Make rows clickable (with stopPropagation) |
| `dashboard/src/pages/DevRunDetail.tsx` | ID link, PR fix, remove merge |
| `dashboard/src/pages/TraceDetail.tsx` | Cross-link to new detail pages |
| `dashboard/src/pages/SessionDetail.tsx` | Cross-link to new detail pages |
| `dashboard/src/api/llmCalls.ts` | Add `useLlmCall(id)` |
| `dashboard/src/api/toolCalls.ts` | Add `useToolCall(id)` |
| `dashboard/src/api/devRuns.ts` | Remove `useMergeDevRun` |

## Sources

- Issue: [#361](https://github.com/senara-solutions/mika/issues/361)
- Existing detail page pattern: `dashboard/src/pages/DevRunDetail.tsx`
- DB getter pattern: `db.rs::get_task_unscoped`
- Handler pattern: `server/dashboard.rs::handle_task_detail`
- Hook pattern: `dashboard/src/api/tasks.ts::useTask`
- Learnings: `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md`
- Learnings: `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md`
- Learnings: `docs/solutions/integration-issues/openapi-spec-drift-missing-utoipa-annotations.md`
