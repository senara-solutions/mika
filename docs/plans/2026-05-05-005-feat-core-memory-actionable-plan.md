---
title: "feat: Make Agent Detail Core Memory panel actionable"
type: feat
status: active
date: 2026-05-05
issue: 656
parent_plan: docs/plans/2026-05-05-006-feat-core-memory-actionable-plan.md
---

# feat: Make Agent Detail Core Memory panel actionable

## Overview

Transform the Core Memory panel on the Agent Detail page from a truncated inspection-only view into an actionable surface. Sections become expandable to show full content, structured data renders typed instead of raw JSON, token budgets gain color-coded thresholds, and per-section metadata (last updated, edit count) becomes visible.

## Problem Frame

The Core Memory panel proves data exists but doesn't help operators work with it. Content is truncated to ~3 lines with no expansion, WORKFLOWS renders as raw JSON, token budgets lack threshold semantics, and there's no temporal context per section. Operators cannot read full section content, assess budget health at a glance, or understand when sections were last modified.

## Requirements Trace

- R1. Long sections (self_model, etc.) readable in full from the agent detail page without navigating away
- R2. Structured content (WORKFLOWS) rendered as typed list/structure, not raw JSON
- R3. Per-section "last updated" timestamp visible
- R4. Token budget bars with color-coded thresholds (green <60%, amber 60-85%, red >85%)
- R5. `<TokenBudgetBar />` extracted to `@senara-solutions/ui` as a reusable primitive
- R6. Edit counter disambiguated (explain what the numbers mean)
- R7. Facts panel integrated as a tab alongside core memory

## Scope Boundaries

- No inline editing of core memory from the dashboard (read-only is acceptable for now)
- No diff view against previous section states (edit history deferred)
- No cross-pivot between facts and core memory sections (deferred)
- No `<ExpandableContentCard />` extraction to `packages/ui/` — the pattern will be page-local until it crystallizes across more pages

### Deferred to Separate Tasks

- Edit history surfaced (panel, drawer, or link): future iteration after audit_events pagination is designed
- Cross-slice memory pivoting (click fact -> see related section): requires backend enrichment
- `<ContentRenderer />` generic typed-content component: extract after pattern appears in 3+ pages
- `<MemorySection />` composite component: extract after the pattern stabilizes

## Context & Research

### Relevant Code and Patterns

- `dashboard/src/pages/AgentDetail.tsx` — current Core Memory panel (inline `MemoryBlock` and `TokenBar` components)
- `dashboard/src/components/CollapsibleCard.tsx` — existing collapsible pattern with chevron toggle
- `dashboard/src/api/agents.ts` — `CoreMemory` type with `{ key, value, token_count, updated_at }`
- `packages/ui/src/components/MarkdownContent.tsx` — existing markdown renderer
- `packages/ui/src/components/CopyButton.tsx` — copy affordance
- `crates/mika-agent/src/server/dashboard.rs` — `handle_agent_detail` handler, `handle_agent_audit` handler
- `crates/mika-agent/src/db.rs` — `get_all_core_memory()`, `count_core_memory_edits_latest_session()`
- LLM Call Detail page pattern: collapsible reasoning panel with `max-h-96 overflow-y-auto` for long content

### Institutional Learnings

- **ListRow enforcement:** `<ListRow variant="expandable" />` from `@senara-solutions/ui` is mandatory for expandable rows in list contexts. However, Core Memory blocks are cards in a grid, not table rows — ListRow does not apply here.
- **Lifecycle states:** Must use `<LoadingState>`, `<ErrorState>`, `<EmptyState>` from `@senara-solutions/ui` (already done in current code).
- **LLM Call Detail pattern (mika#653):** Collapsible panels use "collapsed by default" for secondary content. Long content uses `<pre>` + CopyButton + `max-h-96 overflow-y-auto`.
- **Core memory data model:** 5 sections, 500-token-per-block cap, DB-backed, `updated_at` already in the API response.
- **Design tokens over hardcoded colors:** Status colors must reference design tokens. Token budget thresholds should use `--color-success`, `--color-warning`, `--color-error`.

### External References

- Stitch screen #4 (`7705e941bd5d4f18adbc43e0d19cac6f`) — Agent Core Memory Widget (canonical)
- Stitch screen #6 (`2e9012604d5b4718b5ab7e055ebb63df`) — Agents Overview & Details
- `docs/design/luminescent-core.md` — rulebook (section labels uppercase tracking-wide, tonal-shift surfaces, no 1px dividers)

## Key Technical Decisions

- **Expand pattern:** Click-to-expand inline on the card (not modal or drill-through). The card grows to show full content with `max-h-96 overflow-y-auto` and a CopyButton. Matches the LLM Call Detail precedent for long-content expansion.
- **TokenBudgetBar in packages/ui:** Extract as a reusable component since it will be needed on the LLM Calls detail page (mika#653) for token usage visualization. Three threshold tiers with design token colors.
- **WORKFLOWS formatting:** Parse as JSON, render as a key-value list. Fallback to pre-formatted text if parsing fails.
- **Facts panel:** Add as a tab within the Core Memory card area (tabs: "Core Memory" | "Facts"). Facts already have an API endpoint via the agent's audit events; a dedicated `/agents/:id/facts` endpoint is needed.
- **No backend changes for core memory:** The `CoreMemory` type already includes `updated_at` and `token_count`. The only backend addition is a facts list endpoint.

## Open Questions

### Resolved During Planning

- **Should expand be modal or inline?** Inline — matches the LLM Call Detail precedent and keeps the operator on the page (R1).
- **Should TokenBudgetBar live in packages/ui or dashboard?** In `packages/ui` — it's needed by mika#653 (LLM Calls) too, and the threshold semantics are generic enough (R5).
- **How to render WORKFLOWS?** Parse JSON, render as definition list with key-value pairs. Graceful fallback to `<pre>` on parse failure.
- **What does "Edits: 0 / 3" mean?** It's `core_memory_edits_this_session` / `EDIT_BUDGET`. The budget is the per-session cap on `update_core_memory` tool calls. Disambiguate by showing "X of Y tool edits this session" with a tooltip.

### Deferred to Implementation

- Exact Tailwind classes for the expanded state animation
- Whether facts endpoint should paginate or return all (likely paginate with small default)

## Implementation Units

- [ ] **Unit 1: TokenBudgetBar component in packages/ui**

**Goal:** Create a reusable token budget progress bar with three color-coded threshold tiers.

**Requirements:** R4, R5

**Dependencies:** None

**Files:**
- Create: `packages/ui/src/components/TokenBudgetBar.tsx`
- Modify: `packages/ui/src/index.ts` (export)
- Test: `packages/ui/src/components/TokenBudgetBar.test.tsx`

**Approach:**
- Three threshold tiers: green (`--color-success`) for <60%, amber (`--color-warning`) for 60-85%, red (`--color-error`) for >85%
- Props: `{ used: number, cap: number, showLabel?: boolean, className?: string }`
- Compute percentage internally, select color tier, render progress bar with optional "used/cap" label
- Use design tokens from `theme.css`, not hardcoded Tailwind color utilities
- ARIA: `role="progressbar"` with `aria-valuenow`, `aria-valuemin`, `aria-valuemax`, `aria-label`

**Patterns to follow:**
- `packages/ui/src/components/StatusBadge.tsx` — variant-based component with design token colors
- Current inline `TokenBar` in `AgentDetail.tsx` — base visual structure to improve upon

**Test scenarios:**
- Happy path: renders green bar at 40% (200/500)
- Happy path: renders amber bar at 75% (375/500)
- Happy path: renders red bar at 92% (460/500)
- Edge case: 0 tokens used renders empty bar with green color
- Edge case: used > cap renders 100% width with red color
- Happy path: `showLabel` renders "200/500" text
- Happy path: correct ARIA attributes set

**Verification:**
- Component renders with correct threshold colors at each tier boundary
- Exported from `@senara-solutions/ui` package index

---

- [ ] **Unit 2: Expandable MemoryBlock with full content**

**Goal:** Make each core memory section expandable to show full content inline, replacing the 3-line truncation.

**Requirements:** R1

**Dependencies:** Unit 1 (TokenBudgetBar)

**Files:**
- Modify: `dashboard/src/pages/AgentDetail.tsx`

**Approach:**
- Add `expanded` state per block (default collapsed)
- Collapsed: keep current 3-line clamp with a "Show more" affordance (chevron or link)
- Expanded: remove clamp, show full content in a scrollable container (`max-h-96 overflow-y-auto`), add CopyButton for the full text
- Use `TokenBudgetBar` from `@senara-solutions/ui` replacing the inline `TokenBar`
- Keyboard accessible: Enter/Space to toggle expand

**Patterns to follow:**
- `dashboard/src/components/CollapsibleCard.tsx` — chevron rotation pattern
- LLM Call Detail reasoning panel — collapsed-by-default with overflow scroll

**Test scenarios:**
- Happy path: clicking a collapsed block expands it showing full content
- Happy path: clicking an expanded block collapses it back to 3-line clamp
- Happy path: expanded block shows CopyButton
- Edge case: short content (< 3 lines) does not show expand affordance
- Happy path: keyboard Enter toggles expansion

**Verification:**
- `self_model` (1800+ chars) is readable in full without leaving the page
- CopyButton copies the full section value

---

- [ ] **Unit 3: WORKFLOWS content formatting**

**Goal:** Render WORKFLOWS section content as a structured key-value list instead of raw JSON.

**Requirements:** R2

**Dependencies:** Unit 2

**Files:**
- Modify: `dashboard/src/pages/AgentDetail.tsx`

**Approach:**
- Add a `ContentRenderer` helper within the page that detects content shape
- For WORKFLOWS (and any JSON-shaped content): parse with `JSON.parse()`, render as a `<dl>` definition list with keys as `<dt>` and values as `<dd>`
- For nested objects: render values as indented sub-entries
- Fallback: if `JSON.parse` fails, render as `<pre>` with monospace font (current behavior but prettier)
- For other sections (plaintext): render as-is with `whitespace-pre-wrap`

**Patterns to follow:**
- `packages/ui/src/components/MarkdownContent.tsx` — content rendering component pattern

**Test scenarios:**
- Happy path: valid JSON object renders as definition list with keys and values
- Happy path: nested JSON renders with indented sub-entries
- Error path: malformed JSON falls back to pre-formatted text
- Happy path: non-WORKFLOWS sections render as whitespace-pre-wrap plaintext
- Edge case: empty value renders placeholder text

**Verification:**
- WORKFLOWS no longer shows raw `{'URL': "Always read..."}` — shows formatted key-value list instead

---

- [ ] **Unit 4: Per-section metadata (updated_at + edit count disambiguation)**

**Goal:** Show "last updated" timestamp per section and clarify the edit counter semantics.

**Requirements:** R3, R6

**Dependencies:** Unit 2

**Files:**
- Modify: `dashboard/src/pages/AgentDetail.tsx`

**Approach:**
- Each `MemoryBlock` shows `updated_at` as relative time (using `formatRelativeTime` from `@senara-solutions/ui`) below the token bar
- Header edit counter changed from "Edits: X / 3 used this session" to "X of 3 tool edits this session" with a title tooltip: "Number of update_core_memory tool calls in the current agent session (budget resets each session)"
- Style the timestamp as `text-[10px] text-muted/40 font-mono`

**Patterns to follow:**
- Agent header already uses `formatRelativeTime` for "Created X ago"

**Test scenarios:**
- Happy path: each section shows "Updated 5m ago" style relative timestamp
- Happy path: header tooltip explains the edit budget semantics
- Edge case: section never updated shows the created_at timestamp
- Happy path: edit counter text is "X of 3 tool edits this session"

**Verification:**
- Per-section "last updated" is visible at a glance
- Edit counter meaning is clear from the label + tooltip

---

- [ ] **Unit 5: Facts panel (backend endpoint + frontend tab)**

**Goal:** Surface the agent's structured facts alongside core memory as a tabbed view.

**Requirements:** R7

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/server/dashboard.rs` (add `handle_agent_facts` endpoint)
- Modify: `crates/mika-agent/src/async_db.rs` (expose facts query)
- Modify: `crates/mika-agent/src/db.rs` (add `get_facts_for_agent` if not existing)
- Modify: `dashboard/src/api/agents.ts` (add `useAgentFacts` hook + `Fact` type)
- Modify: `dashboard/src/pages/AgentDetail.tsx` (add tabs, facts list)

**Approach:**
- Backend: Add `GET /api/v1/agents/:id/facts` returning paginated facts (category, key, value, updated_at). Reuse existing `search_memory` DB query or direct `SELECT * FROM facts WHERE agent_id = ?` with pagination.
- Frontend: Replace the single "Core Memory" card header with a tab bar ("Core Memory" | "Facts (N)"). Tab state managed locally with `useState`.
- Facts tab renders a scrollable list grouped by category, each fact as a compact row showing key + truncated value + timestamp.
- Use `<EmptyState message="No facts stored" />` when empty.
- Use `<LoadingState variant="detail" />` while fetching (conditional query, only when tab active).

**Patterns to follow:**
- `handle_agent_sessions` — paginated sub-resource endpoint pattern
- `useAgentSessions` — conditional query enablement pattern
- Existing tab patterns in other dashboard pages (if any) or simple button-group tabs

**Test scenarios:**
- Happy path: switching to Facts tab shows list of facts grouped by category
- Happy path: each fact shows key, truncated value, and relative timestamp
- Edge case: no facts shows EmptyState
- Happy path: pagination works when >50 facts
- Integration: facts endpoint returns correct data shape from DB

**Verification:**
- Facts tab visible and functional alongside Core Memory
- Operators can see all structured facts for an agent without leaving the detail page

---

- [ ] **Unit 6: Visual polish and threshold integration**

**Goal:** Final pass ensuring TokenBudgetBar thresholds render correctly in context, expand/collapse animations are smooth, and the overall panel meets the luminescent-core rulebook.

**Requirements:** R4 (in-context validation)

**Dependencies:** Units 1-5

**Files:**
- Modify: `dashboard/src/pages/AgentDetail.tsx`

**Approach:**
- Verify TokenBudgetBar renders correct colors at actual agent data values (e.g., 464/500 = 92.8% = red)
- Add subtle transition on expand/collapse (`transition-all duration-200`)
- Ensure section labels remain UPPERCASE tracking-wide per rulebook §3
- Verify surface hierarchy uses `bg-bg` for inner blocks on `bg-bg-card` container (current pattern)
- Remove any hardcoded Tailwind color utilities in favor of design tokens

**Test expectation: none -- styling and visual polish pass, verified by manual inspection**

**Verification:**
- 464/500 token bar renders in red/error color
- 200/500 renders in green/success color
- Expand/collapse has smooth 200ms transition
- No visual regression on the rest of the Agent Detail page

## System-Wide Impact

- **Interaction graph:** The new facts endpoint adds a route to the dashboard router; existing auth middleware applies automatically. TokenBudgetBar is a leaf component with no callbacks.
- **Error propagation:** Facts endpoint errors surface via `<ErrorState>` in the Facts tab only — does not break the Core Memory tab.
- **API surface parity:** No breaking changes to existing endpoints. The `CoreMemory` response type is unchanged. Facts endpoint is additive.
- **Unchanged invariants:** The agent detail endpoint (`GET /api/v1/agents/:id`) response shape is unchanged. The `core_memory_edits_this_session` field continues to work as before — only the label presentation changes.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| WORKFLOWS content format varies between agents | Parse defensively with JSON.parse try/catch, fallback to pre-formatted text |
| Facts table may not exist for all agents | Return empty array from endpoint (not 404), render EmptyState |
| TokenBudgetBar color tokens not defined in theme.css | Verify `--color-success`, `--color-warning`, `--color-error` exist; add if missing |

## Sources & References

- Related code: `dashboard/src/pages/AgentDetail.tsx`, `packages/ui/src/components/`
- Related PRs/issues: #651, #652, #653, #654, #655
- Design: `docs/design/dashboard-stitch-map.md` (screens #4, #6)
- Institutional: `docs/solutions/653-llm-call-detail-response-content-linked-tool-calls.md`
