---
title: Dashboard Expandable List Body Viewer — Lazy-Load Pattern for Large Detail Content
date: 2026-05-06
category: best-practices
module: dashboard, mika-agent
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding inline detail viewers to dashboard list pages where body content is too large for list endpoints
  - Needing to show response text, reasoning, or other large text fields inline without navigating to detail pages
  - List endpoint deliberately omits large columns for performance but the UI needs to surface their presence
tags:
  - dashboard
  - expandable-rows
  - lazy-load
  - list-detail-split
  - llm-calls
  - listrow
  - body-viewer
---

# Dashboard Expandable List Body Viewer — Lazy-Load Pattern for Large Detail Content

## Context

The dashboard LLM Calls page needed to show response text and reasoning inline without requiring navigation to the detail page. The challenge: `response_text` and `reasoning` columns can be up to 50KB each, and the list endpoint deliberately returns `null` for these fields to keep list queries fast (`row_to_llm_call()` omits them; `row_to_llm_call_detail()` includes them). Adding bodies to the list response would defeat the performance optimization.

This pattern generalizes to any dashboard list page where detail content is too expensive for the list endpoint but operators want inline inspection.

## Guidance

### Backend: Boolean Indicators via IS NOT NULL

Add boolean indicator fields to the shared struct rather than including body content in list responses. Use `IS NOT NULL` SQL checks, which are trivially cheap (null-flag check, no column data read):

```sql
-- List query adds two cheap boolean columns
SELECT id, agent_id, ..., created_at,
       response_text IS NOT NULL, reasoning IS NOT NULL
FROM llm_calls ...
```

The deserialization function reads these as `bool` fields at the end of the column list. The detail query derives booleans from the actual column presence:

```rust
// Detail query: derive booleans from the fetched columns
let response_text: Option<String> = r.get(17)?;
let reasoning: Option<String> = r.get(18)?;
Ok(LlmCallRow {
    // ...
    has_response_text: response_text.is_some(),
    has_reasoning: reasoning.is_some(),
    response_text,
    reasoning,
    // ...
})
```

### Frontend: ListRow Variant Switching + Lazy-Load

1. **Conditional row variant**: Rows with body content use `ListRow variant="expandable"`; rows without use `variant="static"`. This gives expandable rows keyboard a11y, ARIA, and chevron for free while non-expandable rows don't show misleading affordances.

2. **Single-expand state**: Track `expandedId: string | null` — only one row expanded at a time. Expanding a new row collapses the previous one.

3. **Lazy-load via existing detail hook**: When expanded, fetch via `useLlmCall(expandedId)` which already returns body content. No new backend endpoint needed. TanStack Query handles lifecycle (cancellation on collapse, caching on re-expand).

4. **Detail row pattern**: Render expanded content as a `<tr>` below the summary row with `<td colSpan={COL_COUNT}>`. This keeps table structure valid.

5. **Body viewer component**: Create a dedicated component (`LlmBodyViewer`) with:
   - **Content-aware rendering**: `JSON.parse()` detection with `JSON.stringify(parsed, null, 2)` for pretty-printing, plain text fallback
   - **Client-side truncation**: 10K character cap with "Showing N of M characters" indicator and "Show all" toggle
   - **Collapsible sections**: Response visible by default, reasoning collapsed by default (matching detail page)
   - **Copy support**: `CopyButton` always copies full untruncated text
   - **Nested button fix**: Separate the collapse toggle `<button>` from the `CopyButton` to avoid `<button>` nesting HTML violations

6. **Preserve detail page navigation**: Include a "View Full Details" link in the expanded content so operators can still reach the full detail page.

## Why This Matters

- **Performance preserved**: List queries stay fast — boolean indicators add negligible cost, body content is never transferred until explicitly requested per-row
- **UX improvement**: Operators scanning 10-20 LLM calls can inspect responses inline instead of clicking into each detail page
- **Pattern reusable**: The same approach works for tool call input/output, session messages, or any other list page with expensive detail columns
- **Accessibility maintained**: Uses the existing `ListRow` expandable variant with keyboard support and ARIA semantics

## When to Apply

- Any dashboard list page where body/detail columns are deliberately omitted from list queries for performance
- When operators need to inspect large text content inline (>1KB per field)
- When an existing detail endpoint already serves the full data

## Examples

The key insight is the three-layer approach:

1. **Backend indicator** (`has_response_text: bool`) — tells the frontend which rows *can* expand
2. **Conditional variant** — only rows with content get the expandable affordance
3. **Lazy detail fetch** — uses the existing detail endpoint, not a new one

This avoids the common anti-patterns:
- Adding truncated previews to the list response (still transfers data for every row)
- Making all rows navigable when some have no detail content
- Creating a dedicated "body" endpoint when the detail endpoint already serves the data

## Related

- `docs/solutions/653-llm-call-detail-response-content-linked-tool-calls.md` — the #653 work that added `response_text`/`reasoning` columns and the detail page rendering
- `docs/solutions/ui-bugs/dashboard-tool-calls-tabular-ux.md` — click-to-expand pattern used in tool call summaries
- `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md` — UTF-8 truncation safety (relevant for any body truncation)
- `docs/solutions/ui-bugs/dashboard-tool-call-list-truncation-at-20-per-turn.md` — silent truncation is a UI lie; always show "N of M"
- `packages/ui/CLAUDE.md` — ListRow canonical primitive enforcement rules
- mika#672 — the issue that implemented this pattern
