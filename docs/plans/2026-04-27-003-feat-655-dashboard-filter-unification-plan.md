---
title: "feat(ui+dashboard): extract <SelectFilter<T> /> + <AgentFilter />, unify agent selection across dashboard"
type: feat
status: active
date: 2026-04-27
origin: senara-solutions/mika#655
---

# Plan — filter unification (`<SelectFilter<T> />` + `<AgentFilter />`) (mika#655)

**Issue:** [mika#655](https://github.com/senara-solutions/mika/issues/655) — `Dashboard > Filters: unify agent selection (free text vs dropdown), extract filter primitives into @senara-solutions/ui`
**Branch:** `feat/655/dashboard-filters-unify-agent-selection-free`
**Type:** feat (Phase 2 primitive in milestone #13)
**Labels:** bug, enhancement, dashboard

## Problem (per issue body)

The same semantic filter ("which agent's data") is implemented two different ways:
- **Sessions** (`Sessions.tsx:59-67`): free-text `<input>` — substring match, no validation, type-and-submit.
- **Timeline / LlmCalls / ToolCalls** (`Timeline.tsx:85-96`, `LlmCalls.tsx:84-95`, `ToolCalls.tsx:78-89`): native `<select>` dropdown with options fetched from `useAgents()`.

Same user intent, two affordances. Body's framing: ship a unified primitive, watch usage, iterate. *"That's just by using it that we can divine what's next."*

## Audit results (verified during planning)

Inventoried filter rows on every list page (Sessions, Timeline, LlmCalls, ToolCalls, DevRuns, TeamRuns, Tasks, Agents, Traces).

### Agent-filter divergence (verified ✅)

| Page | File:line | Shape |
|---|---|---|
| Sessions | `Sessions.tsx:59-67` | `<input type="text">` — free-text, Enter-to-submit |
| Timeline | `Timeline.tsx:85-96` | `<select>` with `useAgents()`-fetched options |
| LlmCalls | `LlmCalls.tsx:84-95` | `<select>` with `useAgents()`-fetched options |
| ToolCalls | `ToolCalls.tsx:78-89` | `<select>` with `useAgents()`-fetched options |
| DevRuns / TeamRuns / Tasks / Agents | — | No agent filter |

**Three pages already use the dropdown pattern; one page (Sessions) is the outlier.** Unification points Sessions toward the dropdown pattern (matches what 3 of 4 pages already do).

### Categorical-filter inventory (hardcoded `<select>` patterns — candidates for `<SelectFilter<T> />`)

| Page | Filter | Options source | File:line |
|---|---|---|---|
| Sessions | `channel_type` | hardcoded `['', 'cli', 'telegram', 'team', 'system', 'delegate']` | `Sessions.tsx:81-91` |
| Timeline | `event_type` | hardcoded `['', 'message', 'audit', 'task']` | `Timeline.tsx:97-107` |
| ToolCalls | `success` | hardcoded `['', 'true', 'false']` (custom labels: All Results / Success / Failed) | `ToolCalls.tsx:90-100` |
| DevRuns | `status` | hardcoded `['', 'pending', 'in_progress', 'blocked', 'completed', 'cancelled', 'failed']` | `DevRuns.tsx:49-59` |
| TeamRuns | `status` | hardcoded `['', 'running', 'completed', 'failed', 'suspended', 'cancelled']` | `TeamRuns.tsx:43-53` |

All five render the same Tailwind classes (`bg-bg border border-white/[0.06] rounded-lg px-3 py-2 text-sm text-muted focus:outline-none focus:border-accent/40`). Pure duplication.

### Free-text filter inventory (deferred)

Free-text inputs share the same styled className but have different semantics (Enter-to-submit vs immediate onChange):
- `session_id` (Sessions), `trace_id` (Timeline), `model` (LlmCalls), `tool_name` (ToolCalls), `team_name` (TeamRuns), `search` (Agents — client-side, not URL-based)

Extracting `<TextFilter />` is plausible but lower-leverage — these are essentially styled `<input>` elements with semantic divergence per page. **Out of scope** for this ticket; flag as a follow-up trigger if styling drift surfaces.

### `useSearchParamsFilter` hook

`dashboard/src/hooks/useSearchParamsFilter.ts` (25 lines) — provides `updateFilter(key, value)` and `setPage(page)`. Used by Sessions, Timeline, LlmCalls, ToolCalls, DevRuns, TeamRuns. The new filter primitives should integrate with this hook (consume `updateFilter` via prop or return value).

### Existing filter primitives in `packages/ui/`

None. Greenfield extraction. Current exports: StatusBadge, Pagination, EmptyState, CopyButton, MarkdownContent, TaskStatusBadge — no filter components.

### Type-ahead / combobox infrastructure

None. No Radix UI, no Headless UI, no cmdk. All native `<select>`. Native `<select>` already supports keyboard prefix-matching (type "m" to jump to first option starting with "m"); for the agent-set sizes typical here (~5-15 agents), this is sufficient. **Type-ahead with custom combobox infrastructure is out of scope; native `<select>` is the v1 implementation.** If usage proves need for richer search, follow-up trigger named in plan.

### Outliers (out of scope)

- **Tasks** (no filter row; section-embedded filters)
- **Agents** (client-side `.filter()`, not URL-based — different paradigm)
- **Traces** (search-only, not a list with filters)
- **Time range filters** (`from`/`to` exist in backend `TimelineFilters` but no UI today; mika#659's scope)

## Approach

Five changes — same shape as mika#654 but for filters.

### Change 1 — Extend `luminescent-core.md` with filter affordance grammar (§5.3)

**File:** `mika/docs/design/luminescent-core.md` (existing rulebook)

Per mika#657 / mika#654 precedent: when a component API extends a silent area of the rulebook, the rulebook must declare the grammar alongside the code. The rulebook today covers status chips (§5), is gaining row-affordance grammar (§5.2 from mika#654), and is silent on filter-affordance grammar.

Add §5.3 declaring:

```markdown
### 5.3 Filter affordance grammar

Dashboard list surfaces use one of two filter primitives. `<SelectFilter<T> />` from `@senara-solutions/ui` is the canonical primitive for categorical selection (one-of-N from a fixed or fetched option set). `<AgentFilter />` is a specialization that fetches agents via `useAgents()` internally. Hand-rolling `<select>` or filter-shaped `<input>` with categorical options outside these primitives is forbidden.

| Primitive | Use for | Options | Example surfaces |
|---|---|---|---|
| `<SelectFilter<T> />` | Categorical filter (one-of-N) where the option set is known | Static array (`{ label, value }`) or fetched array | channel_type, event_type, success, status |
| `<AgentFilter />` | Specialized agent selection (one agent from the active set) | `useAgents()`-fetched | Sessions, Timeline, LlmCalls, ToolCalls |

**Free-text filters** (e.g., session ID lookup, trace ID lookup, free-text search over content) remain native `<input type="text">` with consistent styling. They are not categorical and do not migrate to `<SelectFilter />`. A `<TextFilter />` primitive may emerge if free-text styling drift surfaces.

**Agent selection: exact `agent_id` match is the canonical pattern (per architect Finding 1, first-pass — named design decision).** Three of four pages with agent filters already render a dropdown; `<AgentFilter />` canonicalizes that pattern. Substring or partial match on agent name is not a supported filter affordance. If usage proves substring search is operationally needed, the trigger is `<AutocompleteFilter />` (named follow-up below), not extending `<AgentFilter />`. This decision applies across the dashboard and downstream `@senara-solutions/ui` consumers (mika-cloud).

**Visual contract:**
- Both primitives render a single dropdown with the canonical filter styling (border, rounded-lg, focus ring per design tokens).
- An empty / "all" option is always the first item, labeled to the consumer's preference (`All Channels`, `All Agents`, `All Statuses`).
- Selected value reflects URL state via `useSearchParamsFilter`'s `updateFilter`.

**Keyboard:** native `<select>` keyboard semantics (Tab focuses, Up/Down navigates options, prefix typing jumps to matching option, Esc closes). v1 does not implement custom combobox / type-ahead; if option-set growth or search-required UX surfaces, that's the trigger for an `<AutocompleteFilter />` follow-up.

**ARIA:** `aria-label` describing the filter dimension (e.g., `aria-label="Filter by agent"`). Native `<select>` provides the rest.
```

Net diff: ~30 lines added.

### Change 2 — Build `<SelectFilter<T> />` and `<AgentFilter />` in `@senara-solutions/ui`

**New files:**
- `mika/packages/ui/src/components/SelectFilter.tsx`
- `mika/packages/ui/src/components/AgentFilter.tsx`

**`<SelectFilter />` shape (per architect Finding 2, first-pass — generics dropped):**

```typescript
interface SelectFilterOption {
  label: string
  value: string
}

interface SelectFilterProps {
  ariaLabel: string                          // "Filter by event type"
  value: string                              // current filter value (or '')
  onChange: (value: string) => void          // typically wired to updateFilter from useSearchParamsFilter
  options: SelectFilterOption[]              // includes the "all" option as first item
}
```

Renders `<select>` with the canonical Tailwind classes (consolidated from the five existing callsites).

**No generics on the API.** The runtime API is `value: string` and `options: { label: string; value: string }[]`. URL compatibility forces strings at the consumer boundary (`useSearchParamsFilter` returns and consumes strings); a generic `<SelectFilter<T> />` would let future contributors add non-string option values that silently break URL serialization. Plain string API makes the right thing obvious. (Same shape and same stakes as mika#654 Finding 4 — drop the optional API surface that enables silent divergence.)

**`<AgentFilter />` shape (thin adapter, per `<TaskStatusBadge />` → `<StatusBadge />` precedent from mika#657):**

```typescript
import SelectFilter from './SelectFilter'
import { useAgents } from '../../../dashboard/src/api/agents'  // ⚠ see "API location" below

interface AgentFilterProps {
  value: string                              // agent_id
  onChange: (agentId: string) => void
  emptyLabel?: string                        // defaults to "All Agents"
}

export default function AgentFilter({ value, onChange, emptyLabel = 'All Agents' }: AgentFilterProps) {
  const { data: agents } = useAgents()
  const options = [
    { label: emptyLabel, value: '' },
    ...(agents ?? []).map((a) => ({ label: a.name, value: a.id })),
  ]
  return (
    <SelectFilter
      ariaLabel="Filter by agent"
      value={value}
      onChange={onChange}
      options={options}
    />
  )
}
```

**API location concern:** `<AgentFilter />` needs `useAgents()` to fetch the agent list. `useAgents()` lives in `dashboard/src/api/agents.ts` — but `packages/ui/` cannot depend on `dashboard/` (the dependency arrow points the other way). Two resolution paths:

1. **Inject the agents prop:** `<AgentFilter agents={agents} value={value} onChange={onChange} />`. Consumer calls `useAgents()` itself and passes the result. Simpler but each consumer repeats the hook call (5 callsites = 5x `useAgents()` invocations).

2. **Move `useAgents()` to `@senara-solutions/ui`:** add `dashboard/src/api/agents.ts` equivalent inside `packages/ui/src/hooks/useAgents.ts`. Requires `packages/ui/` to gain a dependency on `@tanstack/react-query` (it doesn't have one today) and to know the API endpoint conventions. Couples library to dashboard's backend.

**Decision: Path 1 (consumer injects `agents` prop).** Simpler factoring; preserves layer separation. Consumers call `useAgents()` themselves; `<AgentFilter />` consumes the data. The 5x duplication of `useAgents()` is acceptable — it's a thin hook call, and React Query caches the result globally so there's no network duplication.

Revised `<AgentFilter />` API:

```typescript
interface AgentSummary {
  id: string
  name: string
}

interface AgentFilterProps {
  agents: AgentSummary[] | undefined         // result of useAgents() in the consumer
  value: string
  onChange: (agentId: string) => void
  emptyLabel?: string
}
```

Net diff: ~50 lines for `SelectFilter.tsx` + ~25 lines for `AgentFilter.tsx` + 2 lines for `index.ts` exports.

### Change 3 — Migrate dashboard filter callsites

**Files (5 list pages):**

| File:line | Current | Migrates to |
|---|---|---|
| `Sessions.tsx:59-67` (agent free-text) | `<input>` with Enter-submit | `<AgentFilter agents={agents} value={filters.agent_id ?? ''} onChange={(v) => updateFilter('agent_id', v)} />` (matches Timeline pattern) |
| `Sessions.tsx:81-91` (channel_type) | hand-rolled `<select>` | `<SelectFilter ariaLabel="Filter by channel" value={filters.channel_type ?? ''} onChange={...} options={[{label:'All Channels', value:''}, {label:'CLI', value:'cli'}, ...]} />` |
| `Timeline.tsx:85-96` (agent_id) | hand-rolled `<select>` + `agents?.map()` | `<AgentFilter agents={agents} value={filters.agent_id ?? ''} onChange={...} />` |
| `Timeline.tsx:97-107` (event_type) | hand-rolled `<select>` | `<SelectFilter ariaLabel="Filter by event type" value={...} onChange={...} options={...} />` |
| `LlmCalls.tsx:84-95` (agent_id) | hand-rolled `<select>` | `<AgentFilter ...>` |
| `ToolCalls.tsx:78-89` (agent_id) | hand-rolled `<select>` | `<AgentFilter ...>` |
| `ToolCalls.tsx:90-100` (success) | hand-rolled `<select>` | `<SelectFilter ariaLabel="Filter by result" value={...} options={[{label:'All Results', value:''}, {label:'Success', value:'true'}, {label:'Failed', value:'false'}]} />` |
| `DevRuns.tsx:49-59` (status) | hand-rolled `<select>` | `<SelectFilter ariaLabel="Filter by status" value={...} options={...} />` |
| `TeamRuns.tsx:43-53` (status) | hand-rolled `<select>` | `<SelectFilter ariaLabel="Filter by status" value={...} options={...} />` |

After migration, every page that rendered `<select>` for categorical filtering or `<input>` for agent filtering uses the shared primitive. Sessions agent-filter migration is the most impactful — switches from free-text to dropdown, matching the existing Timeline/LlmCalls/ToolCalls pattern.

**Sessions UX note:** the change from free-text agent filter to dropdown is a small UX change. Today's free-text behavior (substring match on agent name) becomes exact-match by `agent_id`. This matches what 3 of 4 sibling pages already do, so it's normalization, not regression. PR description must note this user-visible change.

Net diff: ~40-50 lines reduction in dashboard pages (replacing ~10 lines of inline `<select>` + `<option>.map()` with a 1-line component invocation per callsite).

### Change 4 — Update `packages/ui/CLAUDE.md` enforcement table

**File:** `mika/packages/ui/CLAUDE.md` (does not exist yet — mika#663 / #657 / #654 plans seed/extend it).

Add two rows plus a callsite-pattern note (per architect Finding 4 — ergonomic mitigation for path-a):

| Component | Use for | Hand-rolled forbidden | Migration status |
|---|---|---|---|
| `<SelectFilter />` | All categorical filters in dashboard list pages (channel, event type, status, success, etc.) | Yes | Audited clean (mika#655) |
| `<AgentFilter />` | All agent-selection filters | Yes | Audited clean (mika#655) |

**Callsite pattern for `<AgentFilter />`** (named in CLAUDE.md so consumers don't re-derive):

```tsx
// Consumer is responsible for fetching agents. <AgentFilter /> does NOT call useAgents() —
// preserves layer separation; library cannot depend on dashboard's API layer.
const { data: agents } = useAgents()  // query key: ['agents'] — verify cache shape if duplicating
return <AgentFilter agents={agents} value={filters.agent_id ?? ''} onChange={(v) => updateFilter('agent_id', v)} />
```

The query key for `useAgents()` is `['agents']` (per `dashboard/src/api/agents.ts`). React Query caches globally on this key, so 5 callsites of `useAgents()` produce one network request. Migration plan verifies this cache assumption holds — if any consumer uses a different query key or staleTime, caching breaks silently.

Same seed-or-extend logic as previous tickets.

### Change 5 — Future-trigger note: `<TextFilter />` and `<AutocompleteFilter />`

**Plan-level only, no file change.**

Document the follow-up triggers explicitly so future grooming doesn't re-derive them:

1. **`<TextFilter />`** — emerges if free-text input styling drifts across pages. Current 6 free-text inputs (session_id, trace_id, model, tool_name, team_name, Agents-search) all share the same Tailwind classes today, so no drift exists. If a future PR introduces a free-text input with non-canonical styling, that's the trigger.
2. **`<AutocompleteFilter />`** — emerges if a categorical filter's option-set grows beyond what native `<select>` keyboard semantics handle well (typically >50 options) or if substring search becomes a UX requirement. Current agent set is ~5-15 agents; native `<select>` is sufficient. If agent count grows or search-required UX surfaces, that's the trigger.

These are named in the plan and in `packages/ui/CLAUDE.md` (as planned-but-deferred entries) so future work has clear precedents.

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `mika/docs/design/luminescent-core.md` | +30 lines: §5.3 filter affordance grammar |
| 2 | `mika/packages/ui/src/components/SelectFilter.tsx` (new) | +~50 lines, plain `value: string` API (no generics) |
| 2 | `mika/packages/ui/src/components/AgentFilter.tsx` (new) | +~25 lines |
| 2 | `mika/packages/ui/src/index.ts` | +2 lines: export `SelectFilter`, `AgentFilter` |
| 3 | `mika/dashboard/src/pages/Sessions.tsx` | Replace agent free-text input + channel_type select with `<AgentFilter>` + `<SelectFilter>` |
| 3 | `mika/dashboard/src/pages/Timeline.tsx` | Replace agent_id select + event_type select with primitives |
| 3 | `mika/dashboard/src/pages/LlmCalls.tsx` | Replace agent_id select with `<AgentFilter>` |
| 3 | `mika/dashboard/src/pages/ToolCalls.tsx` | Replace agent_id select + success select with primitives |
| 3 | `mika/dashboard/src/pages/DevRuns.tsx` | Replace status select with `<SelectFilter>` |
| 3 | `mika/dashboard/src/pages/TeamRuns.tsx` | Replace status select with `<SelectFilter>` |
| 4 | `mika/packages/ui/CLAUDE.md` | +2 rows in enforcement table (seed-or-extend) |

Estimated diff: ~150-200 lines across 11 files.

## Tests

`@senara-solutions/ui` has no test scaffolding. Verification by:
1. Build: `npm run build --prefix mika/packages/ui` and `npm run build --prefix mika/dashboard`.
2. Visual: dev server, each migrated page renders the same dropdown styling. Sessions specifically — verify the new `<AgentFilter />` dropdown shows the same agents Timeline/LlmCalls/ToolCalls already show.
3. Behavioral: filter selection updates URL `?agent_id=...` correctly via `useSearchParamsFilter`. Page-1 reset on filter change still works.
4. Drift grep: per architect-pattern from mika#654, run three discovery commands.

## Acceptance criteria

- [ ] `mika/docs/design/luminescent-core.md` includes §5.3 declaring filter affordance grammar (`<SelectFilter />` and `<AgentFilter />` with hand-rolled-forbidden rule, free-text exception, **and named design decision: agent selection uses exact `agent_id` match; substring/partial match is not a supported affordance**).
- [ ] `mika/packages/ui/src/components/SelectFilter.tsx` exists with `{ ariaLabel, value, onChange, options }` API. **No generics on the props interface** (per architect Finding 2).
- [ ] `mika/packages/ui/src/components/AgentFilter.tsx` exists with `{ agents, value, onChange, emptyLabel? }` API; thin adapter delegating to `<SelectFilter />`.
- [ ] `mika/packages/ui/src/index.ts` exports both.
- [ ] All 9 filter callsites in audit table are migrated (Sessions agent + channel; Timeline agent + event_type; LlmCalls agent; ToolCalls agent + success; DevRuns status; TeamRuns status).
- [ ] Sessions agent filter is now a dropdown matching Timeline/LlmCalls/ToolCalls; PR description notes the UX change (free-text → dropdown).
- [ ] `grep -rn "<select" mika/dashboard/src/pages/*.tsx` returns zero matches outside what `<SelectFilter />` produces.
- [ ] `grep -rn 'placeholder="Search agent\.\.\."' mika/dashboard/src/pages/*.tsx` returns zero matches (confirms Sessions free-text agent input is removed).
- [ ] `mika/packages/ui/CLAUDE.md` enforcement table lists `<SelectFilter />` and `<AgentFilter />` as audited clean (mika#655).
- [ ] `npm run build` succeeds in `packages/ui/` and `dashboard/`.
- [ ] Visual verification: Sessions/Timeline/LlmCalls/ToolCalls all render the same agent dropdown; categorical filters all render with consistent styling.

## Out of scope

- **`<TextFilter />` extraction** — no current drift; named as follow-up trigger.
- **`<AutocompleteFilter />` / type-ahead** — native `<select>` sufficient for current agent-set size; named as follow-up trigger.
- **Multi-select agent filter** — body explicitly defers ("Ship single-select first, revisit multi once usage reveals the need"). Out of scope.
- **Time range filter (`<TimeRangeFilter />`)** — mika#659's scope.
- **Tasks page filters** — embedded in section queries, not a filter row. Different paradigm; not migrating.
- **Agents page client-side search** — uses local `.filter()`, not `useSearchParamsFilter`. Different paradigm.
- **Traces page search** — single-trace lookup, not a list filter.
- **`useAgents()` migration into `packages/ui/`** — would couple library to dashboard backend; consumers inject `agents` prop instead.
- **Substituting `<select>` for a custom combobox infrastructure** (Radix, Headless UI, cmdk) — adds a dependency without justification at v1.

## Risks

| Risk | Mitigation |
|---|---|
| Sessions UX change (free-text → dropdown) is a behavioral regression for users relying on substring matching | **Named design decision (per architect Finding 1):** §5.3 in luminescent-core.md declares exact `agent_id` match as the canonical pattern; substring/partial match is not a supported affordance. PR description still names the user-visible change, but the architectural treatment is the rulebook clause, not the PR note. Follow-up trigger: if Sessions users push back on losing substring search, that's the trigger for `<AutocompleteFilter />` (named below) — not for re-introducing free-text. |
| `<AgentFilter />` requires consumer to inject `agents` prop, leading to repeated `useAgents()` hook calls | Acceptable — React Query caches globally, no network duplication. Alternative (move `useAgents` to `packages/ui/`) couples library to dashboard backend. |
| Categorical filter labels (e.g., "All Results" / "Success" / "Failed" for ToolCalls success filter) need consumer-controlled labels, not auto-generated from values | `<SelectFilter />`'s `options: { label, value }[]` API already supports this — consumer provides labels. Verified against ToolCalls.tsx:91-98 which uses custom labels. |
| Generic typing on `<SelectFilter<T> />` complicates the API for consumers | The runtime API is `string` (for URL compatibility); generics are optional at the type level. Consumers can use `<SelectFilter />` (default `string`) or `<SelectFilter<MyEnum> />` if they want compile-time checking. Most consumers use default. |
| Native `<select>` styling can't be fully customized cross-browser (especially the dropdown arrow) | Current dashboard already accepts native `<select>` styling — no regression. If full custom styling becomes required, that's the trigger for `<AutocompleteFilter />`. |
| Channel-type / event-type / status options are duplicated across consumers (each page declares its own option array) | Acceptable — these are page-specific concerns; the *primitive* (`<SelectFilter />`) is shared, the *option arrays* live with the page. If a future page reuses an existing option set, it can import from a shared constants file (out of scope here). |

## Sequencing

1. **Change 1 first** (luminescent-core.md §5.3 grammar). Rulebook precedes code.
2. **Change 2 second** (`<SelectFilter />` + `<AgentFilter />`). Implements grammar.
3. **Change 3 third** (migrate 5 list pages). Depends on Change 2.
4. **Change 4 last** (`packages/ui/CLAUDE.md` enforcement table — seed or extend).
5. **Visual + behavioral verification** (run dashboard, screenshot each migrated page, verify URL state updates correctly).
6. **Open PR** cross-referencing mika#655 with screenshots and Sessions UX change note.

## Verification

```bash
# Confirm rulebook extension
grep -c "5.3 Filter affordance grammar" mika/docs/design/luminescent-core.md  # → 1
grep -c "Hand-rolling .*<select>.*forbidden" mika/docs/design/luminescent-core.md  # → 1

# Confirm components exist + exports
test -f mika/packages/ui/src/components/SelectFilter.tsx && echo "OK"
test -f mika/packages/ui/src/components/AgentFilter.tsx && echo "OK"
grep -E "SelectFilter|AgentFilter" mika/packages/ui/src/index.ts  # → 2 lines

# Confirm AgentFilter delegates to SelectFilter (per delegation pattern from #657)
grep -c "import SelectFilter" mika/packages/ui/src/components/AgentFilter.tsx  # → 1
grep -c "<SelectFilter" mika/packages/ui/src/components/AgentFilter.tsx  # → 1

# Confirm dashboard migrations — primary structural drift detector
grep -rn "<select" mika/dashboard/src/pages/*.tsx  # → 0 matches (all migrated)

# Confirm Sessions free-text agent input removed
grep -rn 'placeholder="Search agent\.\.\."' mika/dashboard/src/pages/*.tsx  # → 0 matches

# Three-command pre-commit discovery sweep (per architect Finding 6 — completeness with expected output shapes)
# 1. Hand-rolled <select> in dashboard pages — expected 0 matches (all migrated; out-of-scope cases like Tasks have no <select>)
grep -rn "<select" mika/dashboard/src/pages/*.tsx  # → 0 matches
# 2. Filter-shaped <input> for agent (was Sessions free-text) — expected 0 matches
grep -rn 'placeholder="Search agent\.\.\."' mika/dashboard/src/pages/*.tsx  # → 0 matches
# 3. useAgents callsites — expected exactly 4 (one per page using <AgentFilter />: Sessions, Timeline, LlmCalls, ToolCalls); confirms cache-shared invocation
grep -rn "useAgents" mika/dashboard/src/pages/*.tsx  # → exactly 4 matches (Sessions + Timeline + LlmCalls + ToolCalls)

# Confirm packages/ui/CLAUDE.md lists primitives as audited clean
grep -E "SelectFilter.*Audited clean.*mika#655|AgentFilter.*Audited clean.*mika#655" mika/packages/ui/CLAUDE.md  # → 2 matches

# Build verification
npm run build --prefix mika/packages/ui
npm run build --prefix mika/dashboard
```

## Discovery items (verified during planning)

1. **Agent-filter divergence is 1-of-4, not 50/50.** Sessions is the outlier; Timeline/LlmCalls/ToolCalls already use the dropdown pattern. Unification points Sessions toward the existing pattern, not a new pattern.
2. **All categorical `<select>` filters share identical Tailwind classes** (5 callsites). Pure duplication; primitive extraction collapses them cleanly.
3. **No type-ahead infrastructure exists.** Native `<select>` keyboard semantics (prefix-typing jumps to match) is sufficient for current option-set sizes. Type-ahead is a follow-up trigger, not a v1 requirement.
4. **`<AgentFilter />` must NOT call `useAgents()` directly.** Library can't depend on dashboard's API layer. Consumer-injected `agents` prop preserves layer separation.
5. **`useSearchParamsFilter` hook is shared infrastructure** — not a refactor target, but a well-formed integration point for the new primitives via `onChange={(v) => updateFilter(key, v)}`.
6. **Free-text inputs share styling but not semantics.** `<TextFilter />` is plausible but lower-leverage; named as follow-up trigger.
7. **Tasks / Agents / Traces are out of scope** — different paradigms (section-embedded, client-side, search-only). Plan calls each out.
8. **Pre-commit discovery discipline applied** — three-command sweep in verification block per mika#654 / mika#657 precedent.
