---
title: "feat: Make Core Memory panel actionable on Agent Detail page"
type: feat
status: active
date: 2026-05-05
issue: 656
---

# feat: Make Core Memory panel actionable on Agent Detail page

## Overview

The Agent Detail page's Core Memory panel currently proves data exists but doesn't help operators work with it. Content is truncated to 3 lines with no expansion, token budgets lack threshold warnings, `updated_at` timestamps are fetched but never rendered, the "Edits: 0 / 3" counter is cryptic, and WORKFLOWS content renders as raw text. This plan makes the panel actionable by adding expandable content, token budget thresholds with color coding, per-section timestamps, content-aware rendering, a new `TokenBudgetBar` shared component, a facts panel, and a core-memory-scoped edit history view.

## Problem Frame

Operators see a dashboard panel that displays the five core memory sections but cannot read full content, understand token budget health, see when sections were last modified, or access related memory slices (facts, audit events). The panel is inspection-only when it should be an operational surface.

## Requirements Trace

- R1. Full content of any core memory section readable from the Agent Detail page without navigation
- R2. Structured content (WORKFLOWS, JSON-like values) rendered as formatted markdown, not raw text
- R3. Per-section "last updated" relative timestamp visible on each memory block
- R4. Token budget bars with color-coded thresholds: green (<60%), amber (60-85%), red (>85%)
- R5. `TokenBudgetBar` component extracted to `@senara-solutions/ui` for reuse on LLM Calls detail page
- R6. "Edits: X / 3" counter disambiguated with tooltip or clearer labeling
- R7. Facts panel integrated alongside core memory showing Layer 2 structured facts
- R8. Core-memory-scoped edit history visible (filtered audit events for `update_core_memory`)

## Scope Boundaries

- Read-only — no edit affordances from the dashboard UI (this is fine but should be stated in the UI)
- No cross-pivot between memory slices (clicking a fact to see which core memory section it informed) — deferred to future iteration
- No diff view between historical states of a section — deferred
- No search across memory content — deferred

### Deferred to Separate Tasks

- `TokenBudgetBar` reuse on LLM Calls detail page (#653): separate PR after this component lands
- Cross-slice memory pivoting: future iteration after facts and audit surfaces prove useful

## Context & Research

### Relevant Code and Patterns

- `dashboard/src/pages/AgentDetail.tsx` — current page with `MemoryBlock`, `TokenBar` inline components
- `dashboard/src/api/agents.ts` — `CoreMemory` type already includes `updated_at: string` (unused in UI)
- `dashboard/src/components/CollapsibleCard.tsx` — established expand/collapse pattern (ChevronDown toggle)
- `packages/ui/src/components/MarkdownContent.tsx` — existing markdown renderer (used for soul.md)
- `packages/ui/src/utils/formatTime.ts` — `formatRelativeTime()` for "3h ago" display
- `packages/ui/src/components/StatusBadge.tsx` — 6-variant status indicator (reuse for threshold severity)
- `crates/mika-agent/src/server/dashboard.rs` — `AgentDetailResponse` struct, `handle_agent_audit` endpoint
- `crates/mika-agent/src/db.rs` — `CoreMemoryEntry` (key, value, token_count, updated_at), `AuditEvent` struct, `count_core_memory_edits_latest_session()`, `CORE_MEMORY_SECTIONS`, `MAX_TOKENS_PER_BLOCK = 500`
- `crates/mika-agent/src/db.rs` lines 6143-6470 — Layer 2 facts: `people`, `commitments`, `preferences`, `events` tables
- Design system: `docs/design/luminescent-core.md` (tonal-shift surfaces, uppercase tracking-wide labels), `docs/design/dashboard-stitch-map.md` (screens #4 and #6 are canonical for this widget)

### Institutional Learnings

- `docs/solutions/dashboard-issues/add-restful-detail-pages-pattern.md` — 4-layer backend pattern for new endpoints
- `docs/solutions/architecture-patterns/extract-shared-ui-package.md` — component extraction to `packages/ui/` must include barrel export + version bump
- `docs/solutions/best-practices/design-system-state-catalog-extraction-2026-04-27.md` — use `<LoadingState variant="detail" />`, `<ErrorState />` from `@senara-solutions/ui`
- `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md` — 500-token cap per block is intentional; visualization should reflect this constraint
- `docs/solutions/653-llm-call-detail-response-content-linked-tool-calls.md` — collapsible pattern (collapsed by default, lighter text) is the established way to show expandable content

### External References

- No external research needed — the codebase has strong local patterns for all required changes.

## Key Technical Decisions

- **Expand/collapse per block using inline state, not CollapsibleCard:** The memory blocks are compact cards within the Core Memory panel, not top-level collapsible sections. Use `useState` with a `Set<string>` tracking expanded keys, defaulting to all collapsed. Show 3-line preview (current `line-clamp-3`) when collapsed, full content when expanded. Click the card to toggle.
- **MarkdownContent for all blocks, not just WORKFLOWS:** The `<MarkdownContent />` component handles both markdown and plaintext gracefully (markdown is a superset of plaintext). Applying it uniformly gives WORKFLOWS proper list rendering while preserving readability for plaintext sections.
- **TokenBudgetBar as a shared `packages/ui/` component:** The issue explicitly requests extraction. The component needs `value`, `max`, and optional threshold configuration. Design tokens for color: `--color-success` (<60%), `--color-warning` (60-85%), `--color-error` (>85%).
- **Backend: new facts endpoint, not embedding in AgentDetailResponse:** Facts can be numerous. Adding a separate paginated endpoint `GET /api/v1/agents/:id/facts` follows the established pattern and avoids inflating the main detail response.
- **Backend: new core-memory audit endpoint:** `GET /api/v1/agents/:id/audit?target_key=<section>` — filter the existing audit endpoint by target_key, or add a dedicated `GET /api/v1/agents/:id/core-memory-history` endpoint. The filter approach is simpler — add optional `tool_name` and `target_key` query params to the existing audit handler.
- **Facts as a tab within the Core Memory card:** The issue mentions "tab, panel, or separate page — design decision." A tab within the Core Memory card keeps related memory slices co-located without extra navigation. Two tabs: "Sections" (current view) and "Facts" (Layer 2). Lightweight — no routing change.

## Open Questions

### Resolved During Planning

- **What tables back the "facts" panel?** Four domain tables: `people`, `commitments`, `preferences`, `events`. The `get_all_facts_for_indexing()` method returns `(type, id, content)` tuples from all four — use this as the basis for the facts API endpoint.
- **Does the audit endpoint already support filtering by target_key?** No — the current `handle_agent_audit` has only `page`/`per_page` params. We need to add `tool_name` and `target_key` optional filter params.
- **Where does the 500-token cap come from?** `MAX_TOKENS_PER_BLOCK = 500` in `db.rs` line ~6090. The frontend hardcodes `BLOCK_TOKEN_CAP = 500`. Both are correct.

### Deferred to Implementation

- Exact animation/transition for expand/collapse — use Tailwind's transition utilities, details TBD during implementation
- Whether to show reasoning text from audit events in the history view — depends on data quality in practice

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
Agent Detail Page
├── Header (unchanged)
├── Two-column grid
│   ├── Core Memory Card
│   │   ├── Header: "Core Memory" + Edit budget indicator (clarified)
│   │   ├── Tabs: [Sections] [Facts] [History]
│   │   ├── Tab: Sections (default)
│   │   │   ├── 2x2 grid of MemoryBlock cards (user_summary, self_model, current_priorities, key_people)
│   │   │   │   └── Each: icon + label + updated_at + content (expandable) + TokenBudgetBar
│   │   │   └── Full-width workflows block
│   │   ├── Tab: Facts
│   │   │   └── Grouped by type (People, Commitments, Preferences, Events) with counts
│   │   └── Tab: History
│   │       └── Filtered audit events for tool_name=update_core_memory, most recent first
│   └── Soul.md (unchanged)
├── Recent Audit Events (unchanged)
└── Recent Sessions (unchanged)
```

## Implementation Units

- [x] **Unit 1: TokenBudgetBar shared component in packages/ui/**

**Goal:** Create a reusable token budget bar component with color-coded thresholds, extracted to the shared UI library for cross-page reuse.

**Requirements:** R4, R5

**Dependencies:** None

**Files:**
- Create: `packages/ui/src/components/TokenBudgetBar.tsx`
- Modify: `packages/ui/src/index.ts` (barrel export)
- Modify: `packages/ui/package.json` (version bump)
- Test: `packages/ui/src/components/TokenBudgetBar.test.tsx`

**Approach:**
- Props: `{ value: number, max: number, thresholds?: { warning: number, danger: number }, label?: string, showFraction?: boolean }`. Defaults: `warning=0.6`, `danger=0.85`.
- Compute percentage, select color token based on threshold: `--color-success` below warning, `--color-warning` between warning and danger, `--color-error` above danger.
- Use Tailwind literal class names for each color state — no string interpolation (Tailwind JIT purge rule).
- Render: label (optional), bar track (`bg-white/[0.06] rounded-full`), filled bar with color, fraction text (`value/max` in mono).
- ARIA: `role="meter"`, `aria-valuenow`, `aria-valuemin=0`, `aria-valuemax`.

**Patterns to follow:**
- `packages/ui/src/components/StatusBadge.tsx` — typed variant mapping, literal Tailwind classes
- `packages/ui/src/components/LoadingState.tsx` — ARIA attributes, prop interface with defaults
- Current inline `TokenBar` in `AgentDetail.tsx` — visual starting point

**Test scenarios:**
- Happy path: value=250, max=500 → renders at 50%, green/success color
- Happy path: value=350, max=500 → renders at 70%, warning color
- Happy path: value=450, max=500 → renders at 90%, error/danger color
- Edge case: value=0, max=500 → renders at 0%, green color, no visual bar
- Edge case: value=500, max=500 → renders at 100%, error color
- Edge case: value=600, max=500 → caps at 100% width, error color
- Happy path: custom thresholds `{ warning: 0.5, danger: 0.8 }` respected
- Happy path: `showFraction=false` hides the "250/500" text
- Happy path: ARIA attributes present with correct values

**Verification:**
- Component renders with correct colors at each threshold boundary
- Exported from `packages/ui/src/index.ts`
- `npm run build --prefix packages/ui` succeeds

- [x] **Unit 2: Expandable memory blocks with timestamps and markdown rendering**

**Goal:** Make each memory block expandable to show full content, add relative timestamps, and render content as markdown instead of raw monospace text.

**Requirements:** R1, R2, R3

**Dependencies:** Unit 1 (TokenBudgetBar)

**Files:**
- Modify: `dashboard/src/pages/AgentDetail.tsx`

**Approach:**
- Add `expandedBlocks: Set<string>` state to `AgentDetail` component.
- `MemoryBlock` gains `isExpanded` and `onToggle` props. When collapsed: `line-clamp-3` preview. When expanded: full content with `max-h-96 overflow-y-auto` (same as soul.md).
- Add click handler on the card div with `cursor-pointer` and subtle hover state (`hover:bg-white/[0.02]`).
- Add ChevronDown/ChevronRight icon toggle (12px, `text-muted/40`) next to the section label.
- Replace `<p className="font-mono">` with `<MarkdownContent content={mem.value} />` wrapped in a container. Import from `@senara-solutions/ui`.
- Add `formatRelativeTime(mem.updated_at)` below the section label — e.g., "Updated 3h ago" in `text-[10px] text-muted/40`.
- Replace inline `TokenBar` with `<TokenBudgetBar value={mem.token_count} max={BLOCK_TOKEN_CAP} />` from `@senara-solutions/ui`.

**Patterns to follow:**
- `dashboard/src/pages/LlmCallDetail.tsx` lines 114-135 — inline expand toggle pattern for Reasoning section
- `dashboard/src/pages/AgentDetail.tsx` — soul.md viewer (`max-h-96 overflow-y-auto` + `<MarkdownContent />`)
- `dashboard/src/components/CollapsibleCard.tsx` — ChevronDown toggle animation

**Test scenarios:**
- Test expectation: none — pure visual/interaction change in a page component. Verified via manual screenshot inspection.

**Verification:**
- Each memory block shows 3-line preview with "Updated X ago" timestamp
- Clicking a block expands it to show full content rendered as markdown
- WORKFLOWS content renders structured text (lists, key-value pairs) instead of raw JSON
- TokenBudgetBar shows green/amber/red colors based on usage percentage
- Dashboard builds successfully: `npm run build --prefix dashboard`

- [x] **Unit 3: Clarify edit budget indicator**

**Goal:** Disambiguate the "Edits: 0 / 3 used this session" counter with clearer labeling and a tooltip explaining the semantics.

**Requirements:** R6

**Dependencies:** None

**Files:**
- Modify: `dashboard/src/pages/AgentDetail.tsx`

**Approach:**
- Change label from `"Edits: {editsUsed} / {EDIT_BUDGET} used this session"` to `"{EDIT_BUDGET - editsUsed} edits remaining"` when edits are available, or `"Edit budget used"` when exhausted.
- Add a `title` attribute (native tooltip) explaining: "Core memory can be updated {EDIT_BUDGET} times per conversation session via the update_core_memory tool."
- Use `StatusBadge` for the indicator: `variant="success"` when budget available, `variant="warning"` when 1 remaining, `variant="error"` when exhausted.
- Remove the `Pencil` icon — the StatusBadge provides sufficient visual signaling.

**Patterns to follow:**
- `packages/ui/src/components/StatusBadge.tsx` — variant-driven status indication

**Test scenarios:**
- Test expectation: none — pure labeling/copy change. Verified via manual inspection.

**Verification:**
- Edit counter text is immediately understandable without prior knowledge of the system
- Tooltip provides full explanation on hover
- StatusBadge color reflects budget health

- [x] **Unit 4: Backend — facts endpoint for Layer 2 memory**

**Goal:** Add a paginated API endpoint serving structured facts (People, Commitments, Preferences, Events) for an agent.

**Requirements:** R7

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (new query method)
- Modify: `crates/mika-agent/src/db/async_db.rs` (async wrapper)
- Modify: `crates/mika-agent/src/server/dashboard.rs` (handler + response types)
- Modify: `crates/mika-agent/src/server/mod.rs` (route registration)
- Test: `crates/mika-agent/src/server/dashboard.rs` (inline test module)

**Approach:**
- New DB method: `list_facts_for_dashboard(agent_id, limit, offset) -> Vec<FactEntry>` where `FactEntry` is a new struct with `{ id, fact_type, content, created_at, updated_at }`. Query across all four fact tables using UNION ALL (same pattern as `get_all_facts_for_indexing` but with pagination and timestamps).
- `FactEntry` struct: `{ id: i64, fact_type: String, content: String, created_at: String, updated_at: Option<String> }`. Derive `Serialize + ToSchema`.
- Async wrapper in `async_db.rs`.
- Handler `handle_agent_facts` at `GET /api/v1/agents/:id/facts` with `PaginationQuery`. Returns `PaginatedResponse<FactEntry>`.
- Route alongside existing agent sub-resource routes.

**Patterns to follow:**
- `handle_agent_audit` — same paginated sub-resource pattern
- `get_all_facts_for_indexing` — UNION ALL across fact tables
- `docs/solutions/dashboard-issues/add-restful-detail-pages-pattern.md` — 4-layer pattern

**Test scenarios:**
- Happy path: agent with 3 people, 2 commitments → returns 5 facts with correct `fact_type` labels
- Happy path: pagination — page 1 with per_page=2 returns 2 items and correct total
- Edge case: agent with no facts → returns empty data with total=0
- Edge case: unknown agent_id → returns empty data (no 404 — follows audit endpoint pattern)

**Verification:**
- `GET /api/v1/agents/:id/facts` returns paginated JSON with fact entries
- Each entry has `fact_type` set to "person", "commitment", "preference", or "event"
- `cargo test -p mika-agent` passes
- `cargo clippy -p mika-agent` clean

- [x] **Unit 5: Backend — audit endpoint filtering by tool_name and target_key**

**Goal:** Extend the existing audit endpoint to support optional filtering by `tool_name` and `target_key`, enabling the frontend to fetch core-memory-scoped edit history.

**Requirements:** R8

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (extend query method with optional filters)
- Modify: `crates/mika-agent/src/db/async_db.rs` (update async wrapper signature)
- Modify: `crates/mika-agent/src/server/dashboard.rs` (add query params to handler)
- Test: `crates/mika-agent/src/db.rs` (inline test)

**Approach:**
- Extend `PaginationQuery` (or create `AuditQuery`) with `tool_name: Option<String>` and `target_key: Option<String>`.
- Extend the existing DB method `list_audit_events_paginated` to accept optional filters. Build WHERE clause dynamically with params vec.
- Async wrapper passes through the optional filters.
- Handler reads the new query params and passes to DB.

**Patterns to follow:**
- `handle_sessions_list` — established pattern for optional query param filtering
- `list_audit_events_paginated` — base method to extend

**Test scenarios:**
- Happy path: filter by `tool_name=update_core_memory` returns only core memory edits
- Happy path: filter by `target_key=self_model` returns only self_model edits
- Happy path: both filters combined narrow results correctly
- Happy path: no filters → returns all audit events (backward compatible)
- Edge case: filter with no matching events → returns empty data with total=0

**Verification:**
- `GET /api/v1/agents/:id/audit?tool_name=update_core_memory` returns filtered results
- `GET /api/v1/agents/:id/audit` (no filters) still works identically to current behavior
- `cargo test -p mika-agent` passes

- [x] **Unit 6: Frontend — Facts tab and History tab in Core Memory panel**

**Goal:** Add "Sections | Facts | History" tabs to the Core Memory panel, with a Facts tab showing Layer 2 structured facts and a History tab showing core-memory-scoped audit events.

**Requirements:** R7, R8

**Dependencies:** Unit 4 (facts endpoint), Unit 5 (audit filtering)

**Files:**
- Modify: `dashboard/src/api/agents.ts` (new types + hooks: `useAgentFacts`, updated `useAgentAudit` with filters)
- Modify: `dashboard/src/pages/AgentDetail.tsx` (tab state, Facts tab content, History tab content)

**Approach:**
- **API layer:** Add `FactEntry` type and `useAgentFacts(agentId, page)` hook. Add optional `tool_name` and `target_key` params to `useAgentAudit`.
- **Tab state:** `activeMemoryTab: 'sections' | 'facts' | 'history'` with `useState('sections')`. Tab bar uses `text-[11px] uppercase tracking-wider` tab labels with underline indicator on active tab.
- **Facts tab:** Fetch via `useAgentFacts`. Group by `fact_type`. Show each fact as a compact row with type badge, content preview, and timestamp. Use `<LoadingState variant="detail" />` and `<EmptyState />` for lifecycle states. Paginate with `<Pagination />`.
- **History tab:** Fetch via `useAgentAudit(agentId, historyPage, 10, { tool_name: 'update_core_memory' })`. Show a timeline of edits: section name, before→after diff preview (truncated), timestamp. Paginate.
- **Conditional queries:** Facts and history queries use `enabled: activeMemoryTab === 'facts'` / `'history'` to avoid unnecessary fetching.

**Patterns to follow:**
- `dashboard/src/pages/AgentDetail.tsx` — existing audit events display (before/after with color coding)
- `dashboard/src/api/agents.ts` — `useAgentAudit` hook pattern
- `packages/ui/CLAUDE.md` — mandatory lifecycle state primitives

**Test scenarios:**
- Test expectation: none — page component integration. Verified via manual inspection and dashboard build.

**Verification:**
- Clicking "Facts" tab shows structured facts grouped by type
- Clicking "History" tab shows core memory edit timeline
- Switching tabs does not refetch data for inactive tabs
- Empty states shown when no facts or no edit history
- Dashboard builds successfully: `npm run build --prefix dashboard`

## System-Wide Impact

- **Interaction graph:** No callbacks or middleware affected. Changes are additive — new endpoint + frontend only.
- **Error propagation:** New endpoints follow existing error handling (internal_error helper). Frontend uses `<ErrorState />`.
- **State lifecycle risks:** None — read-only operations.
- **API surface parity:** New `GET /agents/:id/facts` endpoint. Audit endpoint gains backward-compatible optional params.
- **Integration coverage:** Frontend↔backend integration verified by dashboard build + manual testing with running mika-spirit.
- **Unchanged invariants:** Core memory write path, agent loop, tool execution, audit logging — all unchanged. The dashboard is purely a read surface.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| UNION ALL across four fact tables may be slow for agents with many facts | Paginated with LIMIT/OFFSET; these tables are typically small (<1000 rows per agent). Monitor query time in initial deployment. |
| Tab state increases component complexity | Keep tab content as separate sub-components to maintain readability. Conditional query enablement prevents unnecessary fetching. |
| TokenBudgetBar threshold colors may not meet accessibility contrast ratios | Use the existing design system tokens (`--color-success`, `--color-warning`, `--color-error`) which are already approved in the luminescent-core rulebook. |

## Documentation / Operational Notes

- `docs/openapi/mika-spirit.yaml` may need updating to include the new `/agents/:id/facts` endpoint — verify and update if the spec is actively maintained.
- No migration required — all schema tables already exist.
- No new environment variables.

## Sources & References

- Related issue: [#656](https://github.com/senara-solutions/mika/issues/656)
- Sibling tickets: #651, #652, #653, #654, #655
- Design references: Stitch screens #4 (`7705e941bd5d4f18adbc43e0d19cac6f`) and #6 (`2e9012604d5b4718b5ab7e055ebb63df`)
- `docs/design/luminescent-core.md` — design rulebook
- `docs/design/dashboard-stitch-map.md` — Stitch reconciliation map
