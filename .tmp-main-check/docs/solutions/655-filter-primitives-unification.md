---
module: dashboard
tags: [ui-components, filters, design-system, packages-ui]
problem_type: structural-drift
---

# Filter primitives unification (mika#655)

## Problem

The same "filter by agent" affordance was implemented two different ways: free-text input on Sessions (substring match) vs dropdown on Timeline/LlmCalls/ToolCalls (exact match). Additionally, all categorical filters (`<select>` for channel type, event type, status, success) were hand-rolled with identical Tailwind classes across 5 pages — pure duplication.

## Solution

Extracted two filter primitives into `@senara-solutions/ui`:

1. **`<SelectFilter />`** — generic categorical one-of-N dropdown. Props: `{ ariaLabel, value, onChange, options: { label, value }[] }`. Plain string API (no generics) — URL compatibility forces strings at the `useSearchParamsFilter` boundary; a generic API would let future contributors add non-string values that silently break URL serialization.

2. **`<AgentFilter />`** — thin adapter delegating to `<SelectFilter />`. Props: `{ agents, value, onChange, emptyLabel? }`. Consumer injects `agents` prop (via `useAgents()` in the page) — library cannot depend on dashboard's API layer. React Query caches globally on `['agents']` key, so multiple callsites produce one network request.

## Key decisions

- **No generics on SelectFilter.** The runtime API is always string; generics would enable silent URL serialization bugs.
- **Consumer-injected agents prop.** Moving `useAgents()` into `packages/ui/` would couple the library to dashboard's backend. 5x hook calls is acceptable with React Query caching.
- **Agent selection = exact `agent_id` match.** Named design decision in luminescent-core.md §5.3. Sessions free-text was normalization to what 3 of 4 pages already did. Follow-up path for search: `<AutocompleteFilter />`, not re-introducing free-text.
- **Native `<select>`** — no combobox infrastructure (Radix, Headless UI). Sufficient for current option-set sizes (~5-15 agents).

## Files changed

- `packages/ui/src/components/SelectFilter.tsx` — new
- `packages/ui/src/components/AgentFilter.tsx` — new
- `packages/ui/src/index.ts` — exports
- `docs/design/luminescent-core.md` — §5.3 filter affordance grammar
- `packages/ui/CLAUDE.md` — enforcement table + callsite pattern
- 5 dashboard pages migrated (Sessions, Timeline, LlmCalls, ToolCalls, DevRuns, TeamRuns)

## Verification pattern

Three-command drift detection sweep (run before merging filter-related PRs):

```bash
# 1. No hand-rolled <select> in dashboard pages
grep -rn "<select" dashboard/src/pages/*.tsx  # expect 0 matches

# 2. No free-text agent input
grep -rn 'placeholder="Search agent..."' dashboard/src/pages/*.tsx  # expect 0 matches

# 3. useAgents callsites match AgentFilter usage (4 pages)
grep -rn "useAgents" dashboard/src/pages/*.tsx  # expect 4 pages (Sessions, Timeline, LlmCalls, ToolCalls) + Agents page for its own listing
```

## Follow-up triggers

- **`<TextFilter />`** — emerges if free-text input styling drifts across pages (currently uniform).
- **`<AutocompleteFilter />`** — emerges if agent set grows beyond ~20 or search-required UX surfaces.
