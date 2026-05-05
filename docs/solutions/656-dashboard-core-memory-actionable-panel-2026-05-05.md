---
module: dashboard
date: 2026-05-05
problem_type: best_practice
component: tooling
severity: medium
tags:
  - dashboard
  - core-memory
  - agent-detail
  - expandable-content
  - token-budget
  - facts-panel
  - audit-history
  - packages-ui
applies_when:
  - Adding new read-only data panels to dashboard detail pages
  - Surfacing structured agent data across multiple backend tables
  - Extracting reusable visualization components to packages/ui
---

# Dashboard Core Memory Panel — From Inspection-Only to Actionable

## Context

The Agent Detail page displayed core memory sections (USER_SUMMARY, SELF_MODEL, CURRENT_PRIORITIES, KEY_PEOPLE, WORKFLOWS) in truncated 3-line cards with no expansion mechanism. Token budget bars showed usage but lacked threshold warnings. The `updated_at` field was fetched from the API but never rendered. The edit counter "Edits: 0 / 3 used this session" was cryptic. WORKFLOWS content rendered as raw text instead of structured output. Related memory slices (facts from Layer 2, edit audit history) were not accessible from the same page.

## Guidance

### 1. Expandable content blocks with content-aware rendering

Memory blocks use per-block `useState` for expand/collapse with ChevronDown toggle. Content rendering is content-aware:

- **JSON objects** (detected via `JSON.parse`) render as `<dl>` definition lists with uppercase tracking-wide keys
- **Non-JSON** renders via `<MarkdownContent />` from `@senara-solutions/ui` (handles both markdown and plaintext since markdown is a superset)
- **Collapsed state** uses `line-clamp-3` CSS truncation as preview

The `isLong` threshold (120 chars) gates whether the expand toggle appears — short content doesn't need it.

### 2. TokenBudgetBar as a shared component

Extracted to `packages/ui/` as `<TokenBudgetBar />` with:

- Three-tier color thresholds via design tokens: `bg-success` (<60%), `bg-warning` (60-85%), `bg-error` (>85%)
- Literal Tailwind class map (never string interpolation — JIT purge rule)
- ARIA `role="meter"` with `aria-valuenow`, `aria-valuemin`, `aria-valuemax`
- Props: `{ value, max, thresholds?, label?, showFraction? }`

Follows the `StatusBadge` pattern: typed tier mapping with a `Record<Tier, string>` for CSS classes.

### 3. Backend facts endpoint via UNION ALL

`GET /api/v1/agents/:id/facts` aggregates four domain tables (`people`, `commitments`, `preferences`, `events`) into a single paginated `DashboardFact` response. Key patterns:

- **Count query** uses four sub-SELECT `COUNT(*)` summed — avoids UNION ALL overhead for count
- **Data query** uses UNION ALL with consistent column aliasing (`id`, `category`, `key`, `value`, `updated_at`)
- **Timestamps are heterogeneous**: `last_mentioned` (people), `created_at` (commitments/events), `updated_at` (preferences) — sorted by `updated_at DESC` in the outer query
- **NULL-safe value construction**: People fact uses `CASE WHEN ... THEN ... END` instead of `COALESCE(a, '') || COALESCE(': ' || b, '')` to avoid dangling separator when `relationship IS NULL` but `notes IS NOT NULL`

### 4. Audit endpoint filtering with backward-compatible optional params

Extended `handle_agent_audit` to accept `tool_name` and `target_key` optional query params via a new `AuditQuery` struct. The SQL uses `(?N IS NULL OR column = ?N)` pattern for optional filtering — all params are passed statically, no dynamic WHERE clause construction.

Existing callers (agent tools, investigate panel) use the `AsyncDatabase` wrapper methods that pass `None, None` for the new params — fully backward compatible.

### 5. Tab-based memory views with conditional query enablement

Three tabs (Sections, Facts, History) share a single panel. Each tab's data hook uses `enabled: memoryTab === '<tab>'` to prevent fetching inactive tab data. Page state resets to 1 on tab switch via a `handleTabChange` helper.

### 6. Edit budget indicator with StatusBadge

Replaced the cryptic "Edits: 0 / 3 used this session" with `<StatusBadge>` showing remaining edits: `success` when budget available, `warning` when 1 remaining, `error` when exhausted. Native `title` tooltip explains the semantics.

## Why This Matters

Dashboard panels that display data without actionability are noise. Operators need to read full content, understand resource health at a glance (token budgets), see temporal context (when was this last updated?), and access related data slices (what facts does this agent have? what edits happened recently?). Each improvement turns a passive display into an operational surface.

## When to Apply

- When adding detail panels that show truncated content — always add expand/collapse
- When showing resource usage (tokens, budgets, quotas) — always add threshold coloring
- When data spans multiple backend tables — use UNION ALL with pagination, not embedding in the parent response
- When extending existing endpoints with optional filters — use the `(?N IS NULL OR column = ?N)` SQL pattern for backward compatibility

## Examples

**Before (truncated, no expansion):**
```tsx
<p className="line-clamp-3">{mem.value}</p>
<div className="h-1 bg-accent" style={{ width: `${pct}%` }} />
```

**After (expandable, content-aware, threshold-colored):**
```tsx
<ContentRenderer value={mem.value} expanded={expanded} />
<TokenBudgetBar value={mem.token_count} max={500} label="Tokens" />
<span>Updated {formatRelativeTime(mem.updated_at)}</span>
```

## References

- Issue: [#656](https://github.com/senara-solutions/mika/issues/656)
- Plan: `docs/plans/2026-05-05-006-feat-core-memory-actionable-plan.md`
- Related: `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md` — explains why the 500-token cap per block is intentional
- Related: `docs/solutions/dashboard-issues/add-restful-detail-pages-pattern.md` — 4-layer backend pattern for new endpoints
- Related: `docs/solutions/architecture-patterns/extract-shared-ui-package.md` — component extraction to `packages/ui/`
