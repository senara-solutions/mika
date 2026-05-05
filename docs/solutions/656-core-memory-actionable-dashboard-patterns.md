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
  - facts-endpoint
  - token-budget
  - database-scoping
  - packages-ui
applies_when: Adding new per-agent dashboard endpoints or extracting reusable UI components
---

# Dashboard Agent Detail: Core Memory Actionable Patterns (mika#656)

## Context

The Agent Detail page's Core Memory panel was inspection-only — content truncated to 3 lines with no expansion, WORKFLOWS rendered as raw JSON, token budgets lacked threshold semantics, and there was no temporal context per section. Making it actionable required:
1. A new `TokenBudgetBar` component in `@senara-solutions/ui`
2. Expandable memory blocks with content-aware rendering
3. A new backend endpoint for structured facts
4. Tabbed interface (Sections / Facts / History)

## Guidance

### Database scoping for per-agent endpoints

**Critical pattern:** Dashboard endpoints that query per-agent data (facts, core memory, sessions specific to one agent) must use `agent_state.db` (the per-agent SQLite file), NOT `state.dashboard_db` (the shared unscoped connection).

```rust
// WRONG — queries the shared DB which has no per-agent facts
let (data, total) = state.dashboard_db
    .list_facts_paginated_with_count(&agent_id, per_page, offset)
    .await?;

// CORRECT — resolves the agent first, then queries its DB
let agent_state = state.resolve_agent(&agent_id)?;
let (data, total) = agent_state.db
    .list_facts_paginated_with_count(per_page, offset)
    .await?;
```

The `dashboard_db` is for cross-agent queries (timeline, sessions list). Per-agent data lives in individual SQLite files. The compiler won't catch this because both expose `AsyncDatabase` with the same methods.

### TokenBudgetBar extraction pattern

When extracting a new component to `packages/ui`:
1. Use design token colors (`--color-success`, `--color-warning`, `--color-error`) — never hardcoded Tailwind utilities
2. Add ARIA semantics (`role="meter"`, `aria-valuenow`, `aria-valuemin`, `aria-valuemax`)
3. Make thresholds configurable with sensible defaults (60%/85% for token budgets)
4. Export from `packages/ui/src/index.ts` and document in `packages/ui/CLAUDE.md`

### Content-aware rendering in detail pages

For sections that may contain structured data (JSON) or plain text:
1. Try `JSON.parse()` first — if it produces an object, render as `<dl>` definition list
2. Fall back to `<MarkdownContent />` for plain text (handles markdown formatting gracefully)
3. In collapsed state, always use `line-clamp-3` for consistent preview height

### Facts aggregation from multiple tables

The agent's structured facts span four tables (people, commitments, preferences, events). The dashboard endpoint uses a UNION ALL query with consistent column aliasing:

```sql
SELECT id, category, key, value, updated_at FROM (
    SELECT id, 'People' AS category, canonical_name AS key, ... FROM people
    UNION ALL
    SELECT id, 'Commitments' AS category, description AS key, ... FROM commitments
    UNION ALL
    SELECT rowid AS id, 'Preferences' AS category, category AS key, ... FROM preferences
    UNION ALL
    SELECT id, 'Events' AS category, description AS key, ... FROM events
) ORDER BY updated_at DESC LIMIT ? OFFSET ?
```

The `preferences` table uses `rowid` since it has a composite primary key. React keys use `${fact.category}-${fact.id}` to avoid collisions across tables.

## Why This Matters

- The database scoping bug silently returns empty data (HTTP 200 with `[]`) — no error, no crash, just a forever-empty Facts tab. Without awareness of the two-database pattern, this class of bug is easy to introduce and hard to notice.
- Extracting to `packages/ui` enables reuse across dashboard pages (e.g., LLM Calls token usage in mika#653).
- The tabbed Core Memory panel establishes a pattern for other detail pages that aggregate multiple data sources.

## When to Apply

- Adding any new `GET /api/v1/agents/:id/*` endpoint — always check whether the data lives in `dashboard_db` or per-agent DB
- Extracting a visual primitive that will be needed on 2+ dashboard pages — put it in `packages/ui`
- Rendering content of unknown structure — use the try-JSON-then-markdown pattern

## References

- PR: mika#656
- Design: `docs/design/dashboard-stitch-map.md` (screens #4, #6)
- Related: `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md`
- Related: `docs/solutions/653-llm-call-detail-response-content-linked-tool-calls.md`
