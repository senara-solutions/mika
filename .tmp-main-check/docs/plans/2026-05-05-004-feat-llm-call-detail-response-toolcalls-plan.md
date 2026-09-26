---
title: "feat: Add response content, linked tool calls, and error state to LLM Call detail page"
type: feat
status: active
date: 2026-05-05
---

# feat: Add response content, linked tool calls, and error state to LLM Call detail page

## Overview

The LLM Calls detail page (`/dashboard/llm-calls/<id>`) currently displays metadata (provider, model, tokens, latency) but not the actual LLM response content, linked tool calls, or proper error state rendering. This change adds response text persistence (schema v30→v31), a linked tool calls section, and enhanced error display — the "bug-fix class" items from mika#653.

## Problem Frame

When debugging an LLM call, the most important data is what the LLM responded and what tool calls it triggered. The current page is a metadata-only row printout. Users must manually cross-reference session messages and tool_calls tables via trace_id to see actual content. This defeats the purpose of having a detail page.

## Requirements Trace

- R1. LLM response content visible on the detail page (collapsible if long)
- R2. Tool calls triggered by this LLM call listed with tool_name, success, latency, and navigation links
- R3. Error state renders `error_message` prominently with status styling when `status == 'error'`
- R4. Use `<CopyButton />` from `@senara-solutions/ui` for any copy affordance (constraint from mika#665 AC3)

## Scope Boundaries

- **Prompt content (input messages) is explicitly out of scope** — storing the full prompt (system + conversation history) is expensive and requires design work. The branch name `feat/653/dashboard-llm-calls-detail-no-prompt` codifies this exclusion.
- Token Usage visualization (charts, cost estimates, cache %) — deferred to Stitch design session
- TraceIdWidget as a shared component — deferred to mika#652 design iteration
- Next/prev step navigation — deferred to design iteration
- Skill Variants context/link-through — deferred to design iteration

## Context & Research

### Relevant Code and Patterns

- `dashboard/src/pages/ToolCallDetail.tsx` — canonical pattern for detail pages with content panels (Input/Output sections using `<pre>` + `<CopyButton>`)
- `dashboard/src/pages/LlmCallDetail.tsx` — current page to modify
- `dashboard/src/api/llmCalls.ts` — `useLlmCall(id)` hook returning `LlmCallRow`
- `dashboard/src/api/toolCalls.ts` — `ToolCallRow` interface already declares `llm_call_id`
- `crates/mika-agent/src/agent.rs` line 912–967 — `save_llm_call()` call site where `LlmResponse.content` and `.reasoning` are available but not persisted
- `crates/mika-agent/src/db.rs` — `save_llm_call()` function (line ~5232), `LlmCallRow` struct (line ~446), `get_llm_call_by_id()` (line ~5603)
- `crates/mika-agent/src/server/dashboard.rs` — `handle_llm_call_detail` handler (line ~882) currently returns raw `LlmCallRow`

### Institutional Learnings

- `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md` — confirms `llm_calls` stores metadata only; no prompt/response columns exist
- `docs/solutions/ui-bugs/strip-internal-metadata-tags-from-display.md` — `strip_internal_tags()` must be applied to any LLM response text before display
- `docs/solutions/runtime-errors/utf8-byte-slicing-panic-in-dashboard-dto.md` — use `truncate_chars()` for any string truncation (never byte-slicing)
- `docs/solutions/best-practices/design-system-state-catalog-extraction-2026-04-27.md` — use canonical `LoadingState`/`ErrorState`/`EmptyState` from `@senara-solutions/ui`

## Key Technical Decisions

- **Store response text, not prompt:** The full prompt is the entire messages array (system + history, often 50KB+). The response is typically much smaller. Store response only; prompt display is a future feature requiring separate design.
- **50KB cap on response_text:** Mirrors the existing `tool_calls.input`/`output` cap. Use `truncate_chars()` for safe truncation.
- **Serialize response as text concatenation:** `LlmResponse.content` is `Vec<LlmResponseContent>` (Text + ToolCall variants). Serialize text blocks joined by newlines, and append `[Tool Call: name(args)]` summaries for tool-use responses. This gives a human-readable single text field.
- **Separate `response_text` column (not JSON):** Keeps the common read path simple — most consumers want the text, not structured content blocks.
- **New endpoint for linked tool calls:** `GET /api/v1/llm-calls/{id}/tool-calls` rather than embedding them in the detail response. Keeps the detail payload small and follows the existing pattern (`/traces/{id}/tool-calls`, `/sessions/{id}/tool-calls`).
- **Schema v30→v31:** Single additive migration adding `response_text TEXT` column to `llm_calls`. No table rebuild needed — `ALTER TABLE ADD COLUMN` is safe in SQLite.

## Open Questions

### Resolved During Planning

- **Should we apply `strip_internal_tags()` before storage or at display time?** → Before storage. Stripping at persistence boundary ensures no internal XML leaks to any consumer (dashboard, API clients, future exports). Consistent with the `scrub_secrets()` pattern on `tool_calls`.
- **Should `reasoning` (extended thinking) be stored?** → Yes, as a separate nullable `reasoning TEXT` column. It's useful for debugging and small compared to prompts.

### Deferred to Implementation

- Exact formatting of ToolCall content blocks in the serialized `response_text`
- Whether `max_h_96` overflow is sufficient for very long responses or needs adjustment

## Implementation Units

- [ ] **Unit 1: Schema migration v30→v31 — add response columns**

**Goal:** Add `response_text TEXT` and `reasoning TEXT` nullable columns to `llm_calls` table.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (migration function, SCHEMA_VERSION bump, LlmCallRow struct)

**Approach:**
- Increment `SCHEMA_VERSION` from 30 to 31
- Add `migrate_v30_to_v31()`: two `ALTER TABLE llm_calls ADD COLUMN` statements
- Add fields to `LlmCallRow` struct: `response_text: Option<String>`, `reasoning: Option<String>`
- Update `row_to_llm_call()` deserialization to read the new columns

**Patterns to follow:**
- v21→v22 migration (`ALTER TABLE messages ADD COLUMN internal INTEGER NOT NULL DEFAULT 0`) — same additive ALTER pattern
- `LlmCallRow` struct already has nullable fields (`cache_read_tokens`, `error_message`)

**Test scenarios:**
- Happy path: migration runs on a fresh v30 database, schema_version becomes 31, columns exist
- Happy path: `LlmCallRow` deserializes correctly with NULL values in new columns (backwards compat with pre-migration data)
- Edge case: migration is idempotent if columns already exist (SQLite `ALTER TABLE ADD COLUMN` fails if exists — handle gracefully)

**Verification:**
- `cargo test -p mika-agent` passes
- `SCHEMA_VERSION == 31` in code

---

- [ ] **Unit 2: Persist response content at save_llm_call() call site**

**Goal:** Serialize LLM response content and reasoning, then persist to the new columns.

**Requirements:** R1

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (save_llm_call signature + INSERT statement)
- Modify: `crates/mika-agent/src/agent.rs` (call site — pass response content)

**Approach:**
- Add `response_text: Option<&str>` and `reasoning: Option<&str>` parameters to `save_llm_call()`
- Update the INSERT statement to include the new columns
- At the call site in `agent.rs`, serialize `resp.content` to a string:
  - Join `Text(s)` blocks with newlines
  - Append `[Tool Call: {name}({truncated_args})]` for ToolCall variants
  - Apply `strip_internal_tags()` before storage
  - Apply `truncate_chars(50_000)` cap
- Pass `resp.reasoning.as_deref()` for the reasoning column
- For error cases (Err branch), pass `None` for both

**Patterns to follow:**
- `save_tool_call()` applies `scrub_secrets()` at the persistence boundary
- `tool_calls` table has 50KB cap enforced via helper

**Test scenarios:**
- Happy path: successful LLM call stores response_text with joined text blocks
- Happy path: successful call with reasoning stores reasoning text
- Happy path: error LLM call stores NULL for response_text and reasoning
- Edge case: response with ToolCall content blocks serializes tool summaries
- Edge case: response text exceeding 50KB is truncated safely (char boundary)
- Edge case: response containing internal XML tags (`<context>`, `<task-health>`) has them stripped

**Verification:**
- Unit test confirms round-trip: save → get_by_id returns expected text
- `cargo clippy` passes

---

- [ ] **Unit 3: Add get_tool_calls_by_llm_call_id() DB function + API endpoint**

**Goal:** Enable querying tool calls linked to a specific LLM call via the existing `tool_calls.llm_call_id` FK.

**Requirements:** R2

**Dependencies:** None (independent of Units 1-2)

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (new query function)
- Modify: `crates/mika-agent/src/server/dashboard.rs` (new handler)
- Modify: `crates/mika-agent/src/server/mod.rs` (route registration)

**Approach:**
- Add `get_tool_calls_by_llm_call_id(llm_call_id: &str) -> Result<Vec<ToolCallRow>>` — simple SELECT using the existing `idx_tool_calls_llm_call` index
- Add `handle_llm_call_tool_calls` handler: `Path(id)` → query → return `Vec<ToolCallRow>` as JSON
- Register route: `GET /api/v1/llm-calls/:id/tool-calls`

**Patterns to follow:**
- `query_tool_calls_by_trace()` — same return type and row deserialization
- `handle_trace_tool_calls()` handler pattern in dashboard.rs
- Route pattern: `/api/v1/traces/:id/tool-calls` → analogous `/api/v1/llm-calls/:id/tool-calls`

**Test scenarios:**
- Happy path: LLM call with 3 linked tool calls returns all 3 in order
- Happy path: LLM call with no linked tool calls returns empty array
- Edge case: non-existent LLM call ID returns empty array (not 404 — consistent with trace endpoint behavior)

**Verification:**
- `cargo test -p mika-agent` passes
- Endpoint responds with expected JSON shape

---

- [ ] **Unit 4: Frontend — response content panel**

**Goal:** Display LLM response text and reasoning on the detail page with collapsible sections.

**Requirements:** R1, R4

**Dependencies:** Unit 1 (schema), Unit 2 (data persistence)

**Files:**
- Modify: `dashboard/src/api/llmCalls.ts` (add fields to `LlmCallRow` interface)
- Modify: `dashboard/src/pages/LlmCallDetail.tsx` (add Response and Reasoning sections)

**Approach:**
- Add `response_text: string | null` and `reasoning: string | null` to the `LlmCallRow` TypeScript interface
- Add a "Response" card section after Call Details, following the ToolCallDetail Input/Output pattern:
  - `<pre>` block with `whitespace-pre-wrap break-all max-h-96 overflow-y-auto`
  - `<CopyButton text={call.response_text} />` header
  - Only render when `response_text` is non-null
- Add a "Reasoning" card section (collapsible, collapsed by default) when `reasoning` is non-null
  - Uses same pre/copy pattern
  - `text-muted/50` styling to visually distinguish from primary response

**Patterns to follow:**
- `ToolCallDetail.tsx` lines 132-156 — Input/Output card sections with CopyButton and pre block

**Test scenarios:**
- Happy path: response_text renders in a pre block with copy button
- Happy path: reasoning renders in a collapsible section
- Edge case: null response_text — section not rendered (no empty card)
- Edge case: null reasoning — section not rendered
- Edge case: very long response — scrollable with max-height

**Verification:**
- `npm run build --prefix dashboard` succeeds
- Visual inspection: response text visible, copy button works

---

- [ ] **Unit 5: Frontend — linked tool calls section**

**Goal:** Display tool calls triggered by this LLM call in a dedicated section with navigation links.

**Requirements:** R2, R4

**Dependencies:** Unit 3 (API endpoint)

**Files:**
- Modify: `dashboard/src/api/llmCalls.ts` (add `useLlmCallToolCalls` hook)
- Modify: `dashboard/src/pages/LlmCallDetail.tsx` (add Tool Calls section)

**Approach:**
- Add `useLlmCallToolCalls(llmCallId: string)` query hook: `GET /api/v1/llm-calls/${id}/tool-calls` → `ToolCallRow[]`
- Add "Tool Calls" card section after Response panel:
  - List each tool call as a row with: tool_name (mono), StatusBadge (success/error), latency, Link to `/tool-calls/{id}`
  - Show count in section header: "Tool Calls (3)"
  - Only render when the query returns non-empty results
  - Use conditional query: `enabled: !!call?.id`

**Patterns to follow:**
- `ToolCallDetail.tsx` references section (LLM Call link) — same link pattern in reverse
- `StatusBadge` for success/error indication
- Conditional section rendering (only when data exists)

**Test scenarios:**
- Happy path: 3 linked tool calls render as clickable rows with names, status badges, and latency
- Happy path: 0 linked tool calls — section not rendered
- Edge case: tool call with error_message shows error badge
- Edge case: loading state while fetching tool calls (inline spinner or skeleton)

**Verification:**
- `npm run build --prefix dashboard` succeeds
- Navigating from LLM call detail → tool call detail → back works

---

- [ ] **Unit 6: Frontend — enhanced error state rendering**

**Goal:** When an LLM call has `status: 'error'`, render the error prominently with a dedicated error panel.

**Requirements:** R3

**Dependencies:** None (can be done independently)

**Files:**
- Modify: `dashboard/src/pages/LlmCallDetail.tsx` (enhanced error rendering)

**Approach:**
- When `call.status === 'error'`, render a prominent error card between the header and Call Details:
  - Red/error-tinted border (`border-red-400/20`)
  - Error icon + "LLM Call Failed" heading
  - `error_message` in a pre block (may be multi-line stack traces)
  - `CopyButton` for the error message
- Remove the inline `error_message` MetadataRow from Call Details (now redundant)
- Adjust header `StatusBadge` to be more visually prominent on error

**Patterns to follow:**
- `ErrorState` component styling from `@senara-solutions/ui` for color tokens
- ToolCallDetail error_message rendering (inline red text) — we're upgrading from this pattern

**Test scenarios:**
- Happy path: error call shows prominent red error card with full error_message
- Happy path: success call does NOT show the error card
- Edge case: very long error_message (e.g., full stack trace) — scrollable pre block
- Edge case: null error_message with status='error' — shows generic "Unknown error" text

**Verification:**
- `npm run build --prefix dashboard` succeeds
- Visual inspection: error state is visually distinct and prominent

## System-Wide Impact

- **Interaction graph:** `save_llm_call()` gains two parameters — all callers (just one in `agent.rs`) must be updated. The DB migration runs on next server start.
- **Error propagation:** Migration failure blocks server startup (existing pattern). API endpoint errors return standard 500 JSON.
- **State lifecycle risks:** None — additive schema change. Pre-migration data has NULL in new columns; the frontend gracefully handles null.
- **API surface parity:** The new `/api/v1/llm-calls/:id/tool-calls` endpoint follows the established `/traces/:id/tool-calls` contract exactly.
- **Unchanged invariants:** `MIKA_STORE_LLM_CALLS` gating still applies. `MIKA_LOG_LLM_BODIES` remains separate (dev-only full request/response logging to file). The `LlmCallRow` serialization for list endpoints gains two nullable fields (additive, non-breaking).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Response text storage increases DB size | 50KB cap per field; most responses are <5KB. Same approach as tool_calls which has been running fine. |
| Internal XML tags leaking to dashboard | `strip_internal_tags()` applied at persistence boundary (not display-time). Tested. |
| Schema migration on production DBs | Additive ALTER TABLE — no table rebuild, no data loss, fast execution. |
| Breaking change to `LlmCallRow` serialization | New fields are `Option<String>` — serialize as `null` for old data. TypeScript interface updated with `| null`. Non-breaking for existing consumers. |

## Sources & References

- Related issues: mika#653, mika#652 (team runs), mika#651 (dev runs)
- Related code: `dashboard/src/pages/ToolCallDetail.tsx` (pattern reference)
- Institutional learnings: `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md`
