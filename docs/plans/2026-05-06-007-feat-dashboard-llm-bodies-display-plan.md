---
title: "feat: Display LLM request/response bodies on Dashboard LLM Calls page"
type: feat
status: active
date: 2026-05-06
---

# feat: Display LLM request/response bodies on Dashboard LLM Calls page

## Overview

Add expandable inline body viewers to the LLM Calls list page so operators can inspect full LLM response text and reasoning without navigating to the detail page. Bodies are already stored in SQLite (schema v31, `llm_calls.response_text` and `llm_calls.reasoning` columns, #653) and rendered on the detail page — this feature surfaces them on the list page via expandable rows with lazy-loading.

## Problem Frame

Operators debugging agent behavior need to quickly scan LLM response content across multiple calls. Currently they must click into each individual detail page to see response text and reasoning. For triage workflows where you're scanning 10–20 calls looking for a specific response pattern, this click-per-row friction is significant. The data already exists (schema v31); the gap is purely in the list page UX.

## Requirements Trace

- R1. Expandable rows on the LLM Calls list page that reveal response_text and reasoning inline
- R2. Lazy-load body content per-row (not in the list endpoint — bodies can be 50KB each)
- R3. Boolean indicators in the list response so the frontend knows which rows have bodies before expanding
- R4. JSON syntax highlighting for structured payloads
- R5. Large body truncation with clear "showing N of M characters" indicator
- R6. Copy-to-clipboard for response and reasoning text
- R7. Maintain existing click-to-detail navigation (don't lose the detail page link)

## Scope Boundaries

- No request body display — `request_text` is not stored in `llm_calls` (the request/prompt is assembled in-memory and not persisted). Only `response_text` and `reasoning` are in scope.
- No new database columns or schema migration — all data comes from existing v31 columns.
- No syntax highlighting library — use content-aware rendering (JSON.parse detection → formatted JSON with monospace styling). Full syntax highlighting (e.g., Prism, Shiki) is a future enhancement if needed.
- No changes to the LlmCallDetail page — it already renders response_text and reasoning correctly.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/db.rs` — `row_to_llm_call()` (list, bodies=None) vs `row_to_llm_call_detail()` (detail, bodies populated). The deliberate split must be preserved.
- `crates/mika-agent/src/server/dashboard.rs` — `handle_llm_calls()` (list), `handle_llm_call_detail()` (detail, already returns bodies)
- `dashboard/src/pages/LlmCalls.tsx` — Current list page using `ListRow variant="navigable"`
- `dashboard/src/pages/LlmCallDetail.tsx` — Response/reasoning rendering with `<pre>`, `<CopyButton>`, collapsible reasoning
- `dashboard/src/api/llmCalls.ts` — `useLlmCall(id)` hook already exists for fetching single call with bodies
- `packages/ui/src/components/ListRow.tsx` — Expandable variant with `isExpanded`/`onToggle`, chevron, keyboard a11y

### Institutional Learnings

- **Detail-vs-list split** (from #653 compound): List queries hardcode `response_text: None, reasoning: None` for performance. Must maintain this pattern.
- **UTF-8 truncation safety** (from `utf8-byte-slicing-panic-in-dashboard-dto.md`): Never use `&s[..N]` — always `truncate_chars()` or `floor_char_boundary()`.
- **Silent truncation is a UI lie** (from `dashboard-tool-call-list-truncation-at-20-per-turn.md`): If bodies are truncated, show "N of M" indicator.
- **Content-aware rendering** (from #656): Try `JSON.parse()` for structured display, fall back to pre-formatted text.
- **Click-to-expand pattern** (from `dashboard-tool-calls-tabular-ux.md`): Use `stopPropagation()` on nested interactive elements, `whitespace-pre-wrap break-all` for body content.
- **Internal tags already stripped** (from `strip-internal-metadata-tags-from-display.md`): `strip_internal_tags()` runs before INSERT into `response_text`.

## Key Technical Decisions

- **Lazy-load via existing detail endpoint**: When a row is expanded, fetch via `useLlmCall(row.id)` which already returns bodies. No new backend endpoint needed — just boolean indicators on the list response.
- **Boolean indicators over body previews**: Add `has_response_text: bool` and `has_reasoning: bool` to list responses via cheap `IS NOT NULL` SQL checks. This tells the frontend which rows can be expanded without transferring body content.
- **Expandable rows with detail row pattern**: Switch from `ListRow variant="navigable"` to `ListRow variant="expandable"`. When expanded, render a detail `<tr>` below with body content. A "View Details →" link preserves navigation to the full detail page.
- **Client-side JSON formatting**: Use `JSON.parse()` + `JSON.stringify(parsed, null, 2)` for detected JSON payloads. No external syntax highlighting library — monospace rendering with proper indentation is sufficient for debugging.
- **Client-side body truncation**: Cap display at 10,000 characters with a "Show full response" toggle. The 50K char server-side cap already handles the upper bound. Frontend truncation uses `string.slice(0, N)` (safe — JS strings are UTF-16, no byte-boundary issues).

## Open Questions

### Resolved During Planning

- **Where do bodies come from?** Directly from SQLite `llm_calls.response_text` and `llm_calls.reasoning` columns (schema v31, #653). No Langfuse dependency.
- **Should we use ListRow expandable or a custom expand mechanism?** ListRow expandable — it provides keyboard a11y, ARIA, and chevron for free. The expanded content goes in a separate detail `<tr>`.
- **How to handle the column count mismatch in the detail row?** Use `colspan` spanning all columns for the expanded content area.

### Deferred to Implementation

- Exact `max-h` value for the expanded body container — may need tuning during implementation.
- Whether JSON detection should handle JSONL (one-object-per-line) — start with standard JSON, extend if needed.

## Implementation Units

- [x] **Unit 1: Backend — Add boolean body indicators to list responses**

**Goal:** Add `has_response_text` and `has_reasoning` boolean fields to the LLM calls list endpoint response, enabling the frontend to know which rows have expandable bodies without fetching the body content.

**Requirements:** R3

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs` — Add `has_response_text: bool` and `has_reasoning: bool` to `LlmCallRow`, update `row_to_llm_call()` SQL to include `response_text IS NOT NULL` and `reasoning IS NOT NULL`
- Modify: `dashboard/src/api/llmCalls.ts` — Add `has_response_text` and `has_reasoning` to `LlmCallRow` interface
- Test: `crates/mika-agent/tests/` or inline `#[cfg(test)]` in `db.rs`

**Approach:**
- Add two boolean fields to `LlmCallRow` struct: `has_response_text: bool` and `has_reasoning: bool`
- Modify the list SQL query in `row_to_llm_call()` to SELECT `response_text IS NOT NULL AS has_response_text, reasoning IS NOT NULL AS has_reasoning` — these are index-friendly NULL checks, not full column reads
- The detail query (`row_to_llm_call_detail`) sets both booleans from the presence of the actual columns
- `serde(Serialize)` will include these in the JSON response automatically
- Frontend TypeScript types gain matching optional booleans

**Patterns to follow:**
- Existing `row_to_llm_call()` / `row_to_llm_call_detail()` split pattern
- `cost_usd: Option<f64>` pattern for fields computed outside the SQL query

**Test scenarios:**
- Happy path: LLM call with both response_text and reasoning returns `has_response_text: true, has_reasoning: true` in list response
- Happy path: LLM call with response_text but no reasoning returns `has_response_text: true, has_reasoning: false`
- Edge case: Pre-v31 row (both NULL) returns `has_response_text: false, has_reasoning: false`
- Edge case: Error call (status=error, no response_text) returns `has_response_text: false`

**Verification:**
- `cargo test -p mika-agent` passes
- `cargo clippy` clean
- List endpoint JSON includes the new boolean fields

- [x] **Unit 2: Frontend — Expandable rows with lazy-loaded body content**

**Goal:** Transform the LLM Calls list from click-to-navigate rows to expandable rows that reveal response text and reasoning inline, with lazy-loading of body content on expand.

**Requirements:** R1, R2, R6, R7

**Dependencies:** Unit 1

**Files:**
- Modify: `dashboard/src/pages/LlmCalls.tsx` — Switch to expandable ListRow, add expansion state, detail row rendering, lazy-load hook
- Test: `dashboard/src/pages/LlmCalls.test.tsx` — Expand/collapse behavior tests

**Approach:**
- Add `expandedId` state (`string | null`) to track which row is expanded (single-expand — only one row open at a time)
- Switch `ListRow` from `variant="navigable"` to `variant="expandable"` with `isExpanded={expandedId === row.id}` and `onToggle` toggling the state
- Only render expand chevron for rows where `has_response_text || has_reasoning` is true (rows without bodies stay static or show a disabled state)
- When expanded, render a `<tr>` below with `<td colSpan={colCount}>` containing the body viewer
- Inside the expanded row, call `useLlmCall(expandedId)` to lazy-fetch the detail data (bodies included)
- Show `<LoadingState variant="detail" />` while loading
- Show `<ErrorState>` on fetch failure with retry
- Include a "View Full Details →" link to `/llm-calls/${row.id}` to preserve detail page navigation

**Patterns to follow:**
- `ListRow variant="expandable"` from `packages/ui`
- Lazy-fetch pattern: `useLlmCall(expandedId)` only fetches when `expandedId` is set (already gated by `enabled: !!id`)
- Existing `LoadingState`/`ErrorState` lifecycle state pattern from `packages/ui/CLAUDE.md`

**Test scenarios:**
- Happy path: Clicking a row with `has_response_text=true` expands to show response content
- Happy path: Clicking an expanded row collapses it
- Happy path: Expanding a different row collapses the previous one (single-expand behavior)
- Edge case: Row with `has_response_text=false` and `has_reasoning=false` — no expand affordance shown
- Integration: "View Full Details" link navigates to the correct detail page

**Verification:**
- Expanding a row shows body content after a brief loading state
- Collapsing a row hides the content
- Only one row is expanded at a time
- Rows without bodies don't show expand affordance
- `npm test --prefix dashboard` passes

- [x] **Unit 3: Body content renderer with JSON detection and truncation**

**Goal:** Build the inline body viewer component that renders response_text and reasoning with JSON detection, character truncation, copy support, and proper styling.

**Requirements:** R4, R5, R6

**Dependencies:** Unit 2

**Files:**
- Create: `dashboard/src/components/LlmBodyViewer.tsx` — Inline body viewer component
- Modify: `dashboard/src/pages/LlmCalls.tsx` — Use LlmBodyViewer in the expanded row
- Test: `dashboard/src/components/LlmBodyViewer.test.tsx`

**Approach:**
- Create `LlmBodyViewer` component accepting `responseText: string | null`, `reasoning: string | null`
- **Content-aware rendering:** Try `JSON.parse()` on body text. If valid JSON object/array, display with `JSON.stringify(parsed, null, 2)` for pretty-printing. Fall back to raw text in `<pre>`.
- **Truncation:** Default display cap of 10,000 characters. When truncated, show "Showing 10,000 of N characters" with a "Show all" button. Track `isFullResponse` state per section.
- **Response section:** Always visible when `responseText` is present. Card-style container matching detail page (`bg-bg border border-white/[0.05] rounded-xl p-4`). `<CopyButton>` in header.
- **Reasoning section:** Collapsed by default when `reasoning` is present (matching `LlmCallDetail` pattern). Chevron toggle. `<CopyButton>` visible when expanded.
- **Styling:** `font-mono text-xs text-muted/70 whitespace-pre-wrap break-all max-h-96 overflow-y-auto` (matches detail page pattern). JSON content gets slightly different opacity for visual distinction.

**Patterns to follow:**
- Response/reasoning rendering in `LlmCallDetail.tsx` (lines 98-135)
- Content-aware rendering from #656 (JSON.parse detection)
- `CopyButton` from `@senara-solutions/ui`
- Card styling: `bg-bg border border-white/[0.05] rounded-xl`

**Test scenarios:**
- Happy path: Plain text response renders in pre-formatted block with copy button
- Happy path: Valid JSON response renders pretty-printed
- Happy path: Reasoning section collapsed by default, expands on click
- Edge case: Body longer than 10,000 chars shows truncation indicator with "Show all" toggle
- Edge case: Body is `null` — section not rendered
- Edge case: Invalid JSON (partial JSON, JSONL) falls back to plain text rendering
- Edge case: Empty string body — shows empty state or "No content"
- Integration: CopyButton copies full text (not truncated)

**Verification:**
- JSON payloads display with indentation
- Truncation indicator shows accurate character counts
- "Show all" reveals the full body
- CopyButton works for both truncated and full views
- `npm test --prefix dashboard` passes

- [x] **Unit 4: Update existing tests and add integration coverage**

**Goal:** Update the existing LlmCalls test suite to cover the new expandable behavior and ensure no regressions.

**Requirements:** R1, R2, R3

**Dependencies:** Units 2, 3

**Files:**
- Modify: `dashboard/src/pages/LlmCalls.test.tsx` — Add tests for expand/collapse, lazy-loading, body rendering
- Modify: `dashboard/src/components/LlmBodyViewer.test.tsx` — Component-level tests (created in Unit 3)

**Approach:**
- Update the mock data in `LlmCalls.test.tsx` to include rows with `has_response_text: true` and `has_reasoning: true`
- Add tests for: expand/collapse toggle, loading state during fetch, body content rendering after fetch, "View Full Details" link
- Mock `useLlmCall` to return body data when called with the expanded row's ID
- Verify that rows without bodies don't show the expand affordance

**Patterns to follow:**
- Existing test patterns in `LlmCalls.test.tsx` (mock API hooks, `renderPage` helper, vitest)

**Test scenarios:**
- Happy path: Expand row → loading state → body content appears
- Happy path: "View Full Details" link has correct href
- Edge case: Expanding row with `has_response_text=false, has_reasoning=false` — no expandable affordance
- Edge case: API error on expand → ErrorState with retry button
- Integration: Expand row, verify CostTrendChart still visible (no layout regression)

**Verification:**
- All existing tests continue to pass
- New tests cover the expand/collapse lifecycle
- `npm test --prefix dashboard` passes with no failures

## System-Wide Impact

- **Interaction graph:** The list endpoint gains two boolean fields; the existing detail endpoint is unchanged. The frontend lazy-loads via the existing `useLlmCall(id)` hook — no new API endpoints.
- **Error propagation:** Fetch errors in the expanded row are contained (per-row ErrorState with retry). They don't affect the list rendering or other rows.
- **State lifecycle risks:** Single-expand state ensures only one row fetches at a time. Collapsing a row cancels the TanStack Query subscription (no orphaned fetches).
- **API surface parity:** The boolean indicators are additive — existing API consumers see new fields but are not broken.
- **Unchanged invariants:** The list endpoint continues to return `null` for `response_text` and `reasoning` (performance invariant preserved). The detail endpoint is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Large body rendering causes scroll jank | `max-h-96 overflow-y-auto` constrains the viewport; virtual scrolling is a future enhancement if needed |
| Boolean indicators add marginal SQL cost to list queries | `IS NOT NULL` is trivially cheap — no column data read, just NULL-flag check |
| JSON.parse on malformed data throws | Wrapped in try/catch with plain text fallback — same pattern as #656 |

## Sources & References

- Related code: `crates/mika-agent/src/db.rs` (LlmCallRow, row_to_llm_call, row_to_llm_call_detail)
- Related code: `crates/mika-agent/src/server/dashboard.rs` (handle_llm_calls, handle_llm_call_detail)
- Related code: `dashboard/src/pages/LlmCalls.tsx`, `dashboard/src/pages/LlmCallDetail.tsx`
- Related code: `packages/ui/src/components/ListRow.tsx` (expandable variant)
- Related PRs/issues: #672 (this issue), #653 (LLM call detail page, schema v31), #671 (LLM bodies to Langfuse)
- Institutional learnings: `docs/solutions/653-llm-call-detail-response-content-linked-tool-calls.md`
- Institutional learnings: `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md`
- Institutional learnings: `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md`
