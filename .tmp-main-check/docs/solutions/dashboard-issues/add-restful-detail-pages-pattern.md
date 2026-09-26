---
title: "Adding RESTful detail pages to the dashboard"
category: dashboard-issues
date: 2026-04-01
tags: [dashboard, react, rest, detail-page, api-endpoint, pattern]
issue: 361
---

# Adding RESTful Detail Pages to the Dashboard

## Problem

The observability dashboard had list views for Tasks, LLM Calls, and Tool Calls but no `/:id` detail pages. IDs were not clickable. This broke RESTful conventions — every other entity (sessions, agents, traces, team runs, dev runs) already had detail pages.

## Root Cause

The list pages were built first during the runtime observability feature (#347). Detail pages were deferred and never added.

## Solution

### Backend (4 files)

1. **DB layer** (`db.rs`): Add `get_llm_call_by_id` and `get_tool_call_by_id` following the `get_task_unscoped` pattern — `query_row` with `.optional()` using the existing `row_to_llm_call` / `row_to_tool_call` mappers.

2. **Async wrappers** (`async_db.rs`): One-liner wrappers with `id.to_owned()` + `self.with_db(move |db| ...)`.

3. **Handlers** (`dashboard.rs`): `handle_llm_call_detail` and `handle_tool_call_detail` following the `handle_task_detail` pattern: `Ok(Some(x)) => Json(x)`, `Ok(None) => 404`, `Err(e) => internal_error(e)`.

4. **Routes** (`mod.rs`): `GET /api/v1/llm-calls/{id}` and `GET /api/v1/tool-calls/{id}`.

### Frontend (10 files)

1. **API hooks**: `useLlmCall(id)` and `useToolCall(id)` — `useQuery` with `enabled: !!id`.

2. **Detail pages**: `TaskDetail.tsx`, `LlmCallDetail.tsx`, `ToolCallDetail.tsx` — early-return loading/error/not-found pattern, `MetadataRow` for key-value display, cross-entity links.

3. **Shared component**: Extract `MetadataRow` to `components/MetadataRow.tsx`.

4. **List page links**: Make task labels, LLM call models, and tool call names clickable with `stopPropagation()` where row-level click handlers exist.

5. **Cross-linking**: TraceDetail and SessionDetail sub-tables now link to detail pages.

### Task Detail Full Response

The existing `TaskResponse` truncates `action_config` and `result` to 200 chars for list views. A separate `TaskDetailResponse` returns full fields. The frontend has separate `TaskItem` (list) and `TaskDetailItem` (detail) TypeScript interfaces.

## Key Decisions

- **Separate `TaskDetailResponse` vs extending `TaskResponse`**: Keeps API contract explicit — list endpoints return summaries, detail endpoints return full data.
- **`MetadataRow` extraction**: Eliminates duplication across 4 detail pages without over-abstracting.
- **`stopPropagation` on links**: Preserves expand/collapse behavior on clickable table rows while adding navigation links.
- **Merge endpoint removal**: The `POST /dev-runs/:id/merge` endpoint was removed as unnecessary — the dashboard is a read-only observability surface.

## Prevention

- When adding new list pages to the dashboard, always add the corresponding detail page in the same PR.
- Follow the established 4-layer pattern: DB getter → async wrapper → handler → route.
- Use `MetadataRow` from `components/MetadataRow.tsx` for all detail pages.
- Helper functions (`formatLatency`, `formatTokens`, `sourceBadge`) are duplicated across many files — extract to a shared utils file when adding new pages (see todo 740).
