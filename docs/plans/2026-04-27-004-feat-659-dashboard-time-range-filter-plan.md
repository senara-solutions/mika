---
title: "feat(ui+dashboard+server): add <TimeRangeFilter /> primitive, complete backend time-filter support, migrate every list page"
type: feat
status: active
date: 2026-04-27
origin: senara-solutions/mika#659
---

# Plan — `<TimeRangeFilter />` extraction + dashboard migration + backend completion (mika#659)

**Issue:** [mika#659](https://github.com/senara-solutions/mika/issues/659) — `Dashboard > Time range filter: every observability surface needs one, none have it`
**Branch:** `feat/659/dashboard-time-range-filter`
**Type:** feat (Phase 2 primitive in milestone #13)
**Labels:** enhancement, dashboard
**Stitch reference (per body):** screen `c5b6feddb5444f3d83a7f9b94e140bcd` (Unified Event Timeline Dashboard) — canonical pattern template.

## Problem (per issue body)

Observability is time-scoped. Every dashboard list page that displays time-ordered data should expose a time-range filter. Currently, no page exposes one — users sort by time descending and eyeball.

The body's AC requires:
- `<TimeRangeFilter />` primitive in `@senara-solutions/ui` with quick presets (`15m / 1h / 24h / 7d / 30d / custom`) + custom range picker.
- Every list page exposes it (Sessions, Traces, LLM Calls, Tool Calls, Tasks, Dev Runs, Team Runs, Event Timeline).
- Server-side query enforcement, not client-side row slicing.
- URL-reflected for shareable filtered views.

## Audit results (verified during planning)

### Backend time-filter support — 4-of-8 endpoints fully support, 3 need additions, 1 detail-only

| Endpoint | Support | `from`/`to` fields | SQL WHERE | Action needed |
|---|---|---|---|---|
| `/api/timeline` | ✅ FULL | `TimelineQuery` (`server/dashboard.rs:55-65`) + `TimelineFilters::to_sql` (`db.rs:522-561`) | `created_at >= ? AND created_at <= ?` (ISO 8601 string compare) | Frontend migration only |
| `/api/llm-calls` | ✅ FULL | `LlmCallsQuery` (`server/dashboard.rs:777-787`) + `query_llm_calls` (`db.rs:5249-5256`) | `created_at >= ? AND created_at <= ?` | Frontend migration only |
| `/api/tool-calls` | ✅ FULL | `ToolCallsQuery` (`server/dashboard.rs:824-835`) + `query_tool_calls` (`db.rs:5322-5329`) | `created_at >= ? AND created_at <= ?` | Frontend migration only |
| `/api/team-runs` | ✅ FULL | `TeamRunsQuery` (`server/dashboard.rs:733-741`) + `build_team_run_filter_sql` (`db.rs:8029-8037`) | `r.started_at >= ? AND r.started_at <= ?` | Frontend migration only |
| `/api/sessions` | ❌ NONE | `SessionsQuery` (`server/dashboard.rs:284-293`) + `list_sessions_paginated` (`db.rs:7563-7636`) | None | **Backend addition required** |
| `/api/tasks` | ❌ NONE | `TasksQuery` (`server/dashboard.rs:520-530`) + `build_task_filter_sql` (`db.rs:7916-7960`) | None | **Backend addition required** |
| `/api/dev-runs` | ❌ NONE | `DevRunsQuery` (`server/dashboard_dev_runs.rs:98-103`) + `list_dev_runs_paginated_with_count` (`db.rs:8158-8200`) | None | **Backend addition required** |
| `/api/traces` | — DETAIL-ONLY | No paginated list endpoint; trace detail accessed via timeline filters | N/A | Out of scope (filters via timeline) |

### Latent type-mismatch bug (must fix as part of migration)

`TimelineFilters`, `LlmCallsFilters`, `ToolCallsFilters`, `TeamRunsFilters` all declare `from?: number` and `to?: number` in TypeScript, but the backend SQLite WHERE clauses compare against TEXT (ISO 8601). String-vs-number comparison would never match, so the existing `from`/`to` fields are **non-functional today** — they exist on paper but produce no filtering effect. Verified by reading `dashboard/src/api/{timeline,llmCalls,toolCalls,teams}.ts`.

This is a latent bug in 4 frontend types. Migration must fix it: change `from`/`to` from `number` to `string` (ISO 8601), or add a converter. **Plan chooses string-typed at the API surface** — the URL serialization and SQLite comparison both work with strings; introducing a converter would add an indirection that doesn't pay off.

### Existing `<TimeRangeFilter />`-shaped components

None. Greenfield extraction. No date-picker / time-input components in `dashboard/src/components/` or `packages/ui/src/components/`. No `date-fns`, `dayjs`, or similar in `dashboard/package.json`.

### Stitch reference (assumption, not finalized spec)

Body cites screen `c5b6feddb5444f3d83a7f9b94e140bcd` (Unified Event Timeline Dashboard) as the canonical pattern template. Implementation should match that visual; design has landed (unlike `mika#658` where no Stitch session existed). The architect cannot fetch the Stitch screen from this grooming context, so visual fidelity verification falls to the operator at PR review time.

**Per architect Finding 6 — fallback contract named explicitly:** the proposed shape (5 button presets `15m/1h/24h/7d/30d` + Custom toggle revealing two `<input type="datetime-local">`) is **an assumption, not a finalized spec.** If Stitch screen `c5b6feddb5444f3d83a7f9b94e140bcd` shows a different affordance (e.g., dropdown of presets instead of a button row, or a different preset list), the implementation will be revised to match Stitch before merge. Vincent (operator) checks Stitch fidelity at PR review and approves visual divergence iteration if needed.

This converts a silent risk into a named one — the proposed shape is the architect's best-guess based on body's listed presets, not the visual spec.

### `useSearchParamsFilter` integration

The hook (`dashboard/src/hooks/useSearchParamsFilter.ts`) supports any string key; `<TimeRangeFilter />` consumes it via `onChange={(range) => { updateFilter('from', range.from ?? ''); updateFilter('to', range.to ?? '') }}`. URL state for `?from=...&to=...` is shareable per the body's AC.

### Sibling-ticket overlap

- `packages/ui/CLAUDE.md` shared with mika#663 / #657 / #654 / #655 (seed-or-extend pattern).
- `luminescent-core.md` extension follows mika#657 (§5.1) / mika#654 (§5.2) / mika#655 (§5.3) precedent — §5.4 for time-range affordance grammar.
- `<TimeRangeFilter />` sits next to `<AgentFilter />` and `<SelectFilter />` in filter rows — the migration in `mika#655` and this plan together produce a unified filter-bar shape across the dashboard.

## Approach

Six changes across four layers (rulebook + library + server + dashboard).

### Change 1 — Extend `luminescent-core.md` with §5.4 time-range affordance grammar

**File:** `mika/docs/design/luminescent-core.md`

Per mika#657 / mika#654 / mika#655 precedent — declare grammar before code. Add §5.4:

```markdown
### 5.4 Time-range filter affordance grammar

Time-range filtering is the canonical observability filter. `<TimeRangeFilter />` from `@senara-solutions/ui` is the canonical primitive; hand-rolling `<input type="datetime-local">` or relative-time presets outside this primitive is forbidden on filter rows.

**Affordance shape:**
- Quick presets row: `15m / 1h / 24h / 7d / 30d / Custom`. Selected preset highlights with the focus ring + token color (`text-accent`).
- Custom picker: native `<input type="datetime-local">` for absolute start and end timestamps. Surfaces the timezone of the running system; users entering values in their local time get filtered to that window.
- Empty state: no preset selected = no time filter applied.

**Visual reference:** Stitch screen `c5b6feddb5444f3d83a7f9b94e140bcd` (Unified Event Timeline Dashboard) is the canonical template.

**Format contract:**
- Component emits ISO 8601 strings (e.g., `"2026-04-26T22:00:00Z"`) via the `onChange` callback.
- URL state (via `useSearchParamsFilter`) stores ISO 8601 strings as `?from=...&to=...`.
- Backend SQLite queries compare TEXT columns lexicographically; ISO 8601 ordering matches chronological ordering, so string comparison is correct.
- Frontend types declare `from?: string` and `to?: string` (not `number`). Plan corrects the latent type-mismatch bug across 4 existing API filter types as part of migration.

**Timezone conversion (per architect Finding 1, first-pass — named assumption):**

`<TimeRangeFilter />` emits ISO 8601 UTC strings via `new Date(localInputValue).toISOString()`. This conversion interprets the user's `<input type="datetime-local">` input as the **browser's local timezone** and converts to UTC for backend transmission.

**Known limitation:** users whose browser timezone differs from their intended operational timezone (common for ops engineers working across regions) will see results offset by the difference. The custom-picker shows the value the user typed; the emitted UTC string is offset from it. Result rows are filtered against UTC-stored timestamps.

**Follow-up trigger:** if multi-timezone support is needed (explicit timezone selector in the picker, or preset re-evaluation on timezone switch), add `date-fns-tz` or equivalent. Native `<input type="datetime-local">` does not provide timezone metadata.

The component's inline JSDoc must name this assumption so it's discoverable without reading this rulebook section.

**Server-side enforcement:** filtering happens at the SQL `WHERE` clause level, not by client-side row slicing. List endpoints accept `from`/`to` query params and add `created_at >= ? AND created_at <= ?` (or equivalent timestamp column). Plan adds this support to the 3 endpoints currently missing it (sessions, tasks, dev-runs).

**Keyboard:** preset buttons are focusable (`<button>` semantics — Tab/Enter/Space). Native `<input type="datetime-local">` provides keyboard date entry.

**ARIA:** `role="group"` on the preset row with `aria-label="Time range presets"`. Each preset button has `aria-pressed={isSelected}`. Custom inputs have `aria-label="Start time"` and `aria-label="End time"`.
```

Net diff: ~35 lines.

### Change 2 — Build `<TimeRangeFilter />` in `@senara-solutions/ui`

**New file:** `mika/packages/ui/src/components/TimeRangeFilter.tsx`

```typescript
interface TimeRange {
  from?: string  // ISO 8601, undefined = no lower bound
  to?: string    // ISO 8601, undefined = no upper bound
}

interface TimeRangeFilterProps {
  value: TimeRange
  onChange: (range: TimeRange) => void
  presets?: TimeRangePreset[]  // optional override; defaults to canonical 15m/1h/24h/7d/30d
  ariaLabel?: string           // defaults to "Time range filter"
}

interface TimeRangePreset {
  label: string                // "15m", "1h", "24h", etc.
  durationMs: number           // for computing `from = now - durationMs`
}

const DEFAULT_PRESETS: TimeRangePreset[] = [
  { label: '15m', durationMs: 15 * 60 * 1000 },
  { label: '1h',  durationMs: 60 * 60 * 1000 },
  { label: '24h', durationMs: 24 * 60 * 60 * 1000 },
  { label: '7d',  durationMs: 7 * 24 * 60 * 60 * 1000 },
  { label: '30d', durationMs: 30 * 24 * 60 * 60 * 1000 },
]
```

Render shape:
- Preset row: 5 `<button>` elements (one per preset) + a "Custom" toggle that reveals two `<input type="datetime-local">` fields (start, end).
- Selected preset: button has the canonical focus-ring style (per design tokens) and `aria-pressed="true"`.
- "Clear" affordance: dedicated button or implied by selecting no preset (configurable; default is implicit).
- On preset click: compute `from = ISO(now - durationMs)`, `to = undefined` (open-ended at the upper end), call `onChange`.
- On custom-input change: parse the `datetime-local` string → ISO 8601 → call `onChange`.

**No external date library dependency.** Native `Date.toISOString()` and `<input type="datetime-local">` cover the v1 use case. If timezone handling or relative-date math becomes complex, add `date-fns` (or similar) as a follow-up.

**Inline documentation requirement (per architect Finding 1):** the component's TSDoc / inline comment must include the timezone assumption verbatim:

```typescript
/**
 * <TimeRangeFilter /> emits ISO 8601 UTC strings via new Date(localInputValue).toISOString().
 * This interprets the user's datetime-local input as the browser's local timezone.
 *
 * Known limitation: users whose browser timezone differs from their operational timezone
 * see results offset by the difference. Multi-timezone support is a follow-up trigger;
 * see luminescent-core.md §5.4 for the rulebook clause.
 */
```

Discoverability without reading the rulebook is the goal — both surfaces declare the same constraint.

Export: add `TimeRangeFilter` to `packages/ui/src/index.ts`.

Net diff: ~120 lines for the component (preset buttons + custom picker + ISO conversion + state management) + 1 line for the export.

### Change 3 — Backend: add `from`/`to` support to Sessions, Tasks, DevRuns endpoints

**Files (3 endpoints, 6 sites total):**

#### 3a. Sessions endpoint

**File:** `mika/crates/mika-agent/src/server/dashboard.rs:284-293` (`SessionsQuery`)
- Add `from: Option<String>` and `to: Option<String>` fields.

**File:** `mika/crates/mika-agent/src/db.rs:7563-7636` (`list_sessions_paginated`)
- Add WHERE clauses: `s.started_at >= ?` (when `from` is Some), `s.started_at <= ?` (when `to` is Some).
- Pattern matches existing filters in the same function; ~8 lines added.

#### 3b. Tasks endpoint

**File:** `mika/crates/mika-agent/src/server/dashboard.rs:520-530` (`TasksQuery`)
- Add `from: Option<String>` and `to: Option<String>`.

**File:** `mika/crates/mika-agent/src/db.rs:7916-7960` (`build_task_filter_sql`)
- Add WHERE clauses on `tasks.created_at` (matches the timestamp existing UI ordering uses).
- Decision: filter on `created_at`, not `updated_at`. The dashboard-displayed timestamp is "when the task was created"; filtering by creation time matches user mental model. If future use cases need `updated_at` filtering, that's a separate filter dimension.

#### 3c. DevRuns endpoint

**File:** `mika/crates/mika-agent/src/server/dashboard_dev_runs.rs:98-103` (`DevRunsQuery`)
- Add `from: Option<String>` and `to: Option<String>`.

**File:** `mika/crates/mika-agent/src/db.rs:8158-8200` (`list_dev_runs_paginated_with_count`)
- Add WHERE clauses on `tasks.created_at` (DevRuns are tasks with `trigger_type='manual' AND source IN ('self_dev','github_issue')`; same column).

Net diff: ~30 lines across 3 files (~10 per endpoint).

### Change 4 — Frontend: fix latent type-mismatch correctness bug + add `from`/`to` to new filter types

**Per architect Finding 2 (first-pass):** Change 4 is a **named correctness bug fix**, not a routine type adjustment. Existing `from`/`to` filtering on 4 endpoints (Timeline, LlmCalls, ToolCalls, TeamRuns) is non-functional today because frontend sends numeric epoch values that SQLite TEXT columns compare against lexicographically — strings like `"1714176000000"` never match ISO 8601 like `"2026-04-26T00:00:00Z"`. After this change, the same UI elements that were silently broken will correctly filter results.

**Bug-fix verification (independent of UI migration):**

```
Before fix:
  GET /api/timeline?from=1714176000000&to=1714262400000
  → result set unchanged from no-filter (lexicographic '1...' vs ISO '2...' never matches in range)

After fix (Change 4 type change applied, callsites converted to ISO strings):
  GET /api/timeline?from=2026-04-26T00:00:00.000Z&to=2026-04-27T00:00:00.000Z
  → result set correctly filtered to that 24h window
```

This assertion is independent of `<TimeRangeFilter />` — it can be verified by manual `curl` against the timeline endpoint as soon as Change 4 lands. Plan's verification block names this explicitly.

**Files (7 TypeScript filter types):**

| File | Current | Updates to |
|---|---|---|
| `dashboard/src/api/timeline.ts:14-23` (`TimelineFilters`) | `from?: number; to?: number` | `from?: string; to?: string` (ISO 8601) |
| `dashboard/src/api/llmCalls.ts:24-32` (`LlmCallsFilters`) | `from?: number; to?: number` | `from?: string; to?: string` |
| `dashboard/src/api/toolCalls.ts:23-32` (`ToolCallsFilters`) | `from?: number; to?: number` | `from?: string; to?: string` |
| `dashboard/src/api/teams.ts:49-56` (`TeamRunsFilters`) | `from?: number; to?: number` | `from?: string; to?: string` |
| `dashboard/src/api/sessions.ts:26-32` (`SessionsFilters`) | (no fields) | Add `from?: string; to?: string` |
| `dashboard/src/api/tasks.ts:55-64` (`TasksFilters`) | (no fields) | Add `from?: string; to?: string` |
| `dashboard/src/api/devRuns.ts:25-29` (`DevRunsFilters`) | (no fields) | Add `from?: string; to?: string` |

**Latent bug fixed.** The 4 types that previously had `number` but the backend expected strings now match. No converter layer needed — types and runtime align.

`apiFetch` casts: each type's call to `apiFetch` (e.g., `apiFetch('/llm-calls', filters as Record<string, string | number | undefined>)`) needs verification that the cast still works after the type change. Likely works without modification since `string` is already in the cast union.

Net diff: ~14 lines across 7 files (one or two lines per filter type).

### Change 5 — Migrate dashboard list pages to expose `<TimeRangeFilter />`

**Files (7 list pages — Traces is detail-only, no list to migrate):**

| Page | File | Add to filter row |
|---|---|---|
| Sessions | `Sessions.tsx` | After channel_type filter; wire to `updateFilter('from'/'to', ...)` |
| Timeline | `Timeline.tsx` | In existing filter row, after event_type |
| LlmCalls | `LlmCalls.tsx` | After agent filter |
| ToolCalls | `ToolCalls.tsx` | After success filter |
| Tasks | `Tasks.tsx` | **Tasks page has no filter row today** — add a top-level filter bar above the section dividers. The four sections (WorkItems, TeamRunTasks, StandaloneCallbacks, Scheduled) all consume the time range. **This is a page-level layout change, not a row insertion** (per architect Finding 4) — the filter-bar container is new, page-level URL state is new, and all 4 section queries gain `from`/`to` propagation. PR diff will reflect a layout addition, not just a component plug-in. |
| DevRuns | `DevRuns.tsx` | After status filter |
| TeamRuns | `TeamRuns.tsx` | After status filter |

**Tasks-page UX:** Tasks renders 4 sections without a filter row. Adding `<TimeRangeFilter />` as a page-level filter applies to all 4 sections (each section's `useTasks` query receives the `from`/`to` from URL state). This is a slight UX shift — Tasks gains a filter bar — but it's per the body's AC.

For each callsite: ~5 lines (component invocation + onChange wiring through `useSearchParamsFilter`).

Net diff: ~50 lines across 7 dashboard pages.

### Change 6 — Update `packages/ui/CLAUDE.md` enforcement table

**File:** `mika/packages/ui/CLAUDE.md` (does not exist yet; mika#663/#657/#654/#655 plans seed/extend it).

| Component | Use for | Hand-rolled forbidden | Migration status |
|---|---|---|---|
| `<TimeRangeFilter />` | All time-range filtering on dashboard list surfaces (presets + custom picker, ISO 8601 emission, server-side enforcement) | Yes | Audited clean (mika#659) |

Plus a callsite-pattern note (per mika#655 precedent for ergonomic discoverability):

```tsx
const { searchParams, updateFilter } = useSearchParamsFilter()
const value = { from: searchParams.get('from') ?? undefined, to: searchParams.get('to') ?? undefined }
return <TimeRangeFilter value={value} onChange={(range) => {
  updateFilter('from', range.from ?? '')
  updateFilter('to', range.to ?? '')
}} />
```

Net diff: +1 row in enforcement table + ~10 lines callsite pattern (or seed-shape if file doesn't exist).

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `mika/docs/design/luminescent-core.md` | +35 lines: §5.4 time-range affordance grammar |
| 2 | `mika/packages/ui/src/components/TimeRangeFilter.tsx` (new) | +~120 lines |
| 2 | `mika/packages/ui/src/index.ts` | +1 line: export |
| 3 | `mika/crates/mika-agent/src/server/dashboard.rs` | +4 lines (Sessions + Tasks query structs) |
| 3 | `mika/crates/mika-agent/src/server/dashboard_dev_runs.rs` | +2 lines (DevRuns query struct) |
| 3 | `mika/crates/mika-agent/src/db.rs` | +~25 lines (3 functions: sessions, tasks, dev-runs WHERE additions) |
| 4 | `mika/dashboard/src/api/{timeline,llmCalls,toolCalls,teams}.ts` | Change `number` → `string` in 4 files |
| 4 | `mika/dashboard/src/api/{sessions,tasks,devRuns}.ts` | Add `from?: string; to?: string` in 3 files |
| 5 | `mika/dashboard/src/pages/{Sessions,Timeline,LlmCalls,ToolCalls,Tasks,DevRuns,TeamRuns}.tsx` | Add `<TimeRangeFilter />` to filter row in 7 files; Tasks adds a filter row |
| 6 | `mika/packages/ui/CLAUDE.md` | +1 row + callsite pattern |

Estimated diff: ~300-400 lines across 18 files.

## Tests

Backend: existing tests for `list_sessions_paginated`, `build_task_filter_sql`, and `list_dev_runs_paginated_with_count` should be extended to cover the new `from`/`to` filtering. Pattern: pass `from = ISO(now - 1h)` and `to = ISO(now)`, assert the result set excludes rows outside the range.

Frontend: `@senara-solutions/ui` has no test scaffolding. Verification by:
1. **Build verification** — `npm run build --prefix mika/packages/ui` and `npm run build --prefix mika/dashboard`; `cargo check -p mika-agent`.
2. **Visual verification** — dev server, each migrated page renders the filter bar with `<TimeRangeFilter />`. Click each preset, verify URL updates with `?from=...&to=...`. Verify the result list shrinks accordingly.
3. **Cross-page consistency check** — Sessions / Timeline / LlmCalls / ToolCalls / Tasks / DevRuns / TeamRuns all render the same `<TimeRangeFilter />` shape (5 presets + Custom).
4. **Backend integration test** — manual: hit `/api/sessions?from=2026-04-26T20:00:00Z&to=2026-04-26T22:00:00Z` and verify the result is filtered.
5. **Stitch fidelity check** — operator compares the rendered `<TimeRangeFilter />` against Stitch screen `c5b6feddb5444f3d83a7f9b94e140bcd`. Architect cannot fetch the screen from grooming context; visual fidelity is operator-verified at PR review.

## Acceptance criteria

- [ ] `mika/docs/design/luminescent-core.md` includes §5.4 declaring time-range affordance grammar (presets, custom picker, ISO 8601 format, server-side enforcement, hand-rolled-forbidden rule, **named timezone-conversion assumption per architect Finding 1**).
- [ ] `<TimeRangeFilter />` component's TSDoc/inline comment names the same timezone assumption (discoverable without rulebook lookup).
- [ ] Bug-fix verification artifact: PR includes before/after curl evidence showing `/api/timeline?from=...&to=...` correctly filters with ISO 8601 strings (was non-functional with numeric epochs pre-fix).
- [ ] All `created_at` columns affected by Change 3 are TEXT (verified in db.rs schema) — column-type parity confirmed before WHERE clause additions ship.
- [ ] Tasks page gains a top-level filter-bar container; URL state for `from`/`to` propagates to all 4 section queries.
- [ ] Timezone correctness manual test passes (per Finding 1's named test case): browser TZ change is reflected in emitted ISO strings.
- [ ] `mika/packages/ui/src/components/TimeRangeFilter.tsx` exists with `{ value, onChange, presets?, ariaLabel? }` API and 5 default presets (15m/1h/24h/7d/30d).
- [ ] `mika/packages/ui/src/index.ts` exports `TimeRangeFilter`.
- [ ] Backend `from`/`to` query params are wired through `SessionsQuery`, `TasksQuery`, `DevRunsQuery` and produce `WHERE created_at >= ? AND created_at <= ?` on the corresponding SQL queries.
- [ ] Frontend filter types use `from?: string; to?: string` (ISO 8601), not `number`. The latent type-mismatch bug is fixed.
- [ ] Every list page (Sessions, Timeline, LlmCalls, ToolCalls, Tasks, DevRuns, TeamRuns) exposes `<TimeRangeFilter />`. Tasks gains a top-level filter bar.
- [ ] URL state reflects time-range selection as `?from=...&to=...` and is shareable.
- [ ] `mika/packages/ui/CLAUDE.md` enforcement table lists `<TimeRangeFilter />` as audited clean (mika#659) with callsite pattern.
- [ ] `cargo check -p mika-agent`, `npm run build --prefix mika/packages/ui`, and `npm run build --prefix mika/dashboard` all succeed.
- [ ] Visual verification at PR review: rendered filter matches Stitch screen `c5b6feddb5444f3d83a7f9b94e140bcd`.
- [ ] Backend tests added for the 3 new endpoints' time filtering.

## Out of scope

- **Visual fidelity verification at grooming time** — architect cannot fetch Stitch screens; operator-verified at PR review. Plan acknowledges this.
- **Adding `date-fns` or similar date-handling library** — native `Date.toISOString()` + `<input type="datetime-local">` cover v1. Library added only if a follow-up surfaces complex timezone/relative-date math.
- **Relative time references (e.g., "last build")** — body's presets cover the common cases; complex named ranges are a follow-up trigger.
- **Server-side query optimization for time-range filtering** — adding the WHERE clause is straightforward; index optimization on `created_at` columns is out of scope (pre-existing index status not audited; if performance regresses, that's a follow-up).
- **Traces page list view** — the `/api/traces` endpoint is detail-only. Trace search by time happens via timeline filters (which support time range).
- **Migrating Tasks page's 4-section query architecture** — adding a top-level filter bar passes `from`/`to` through to each section's `useTasks` call; the 4-section structure stays.
- **Custom-picker timezone selector** — native `<input type="datetime-local">` uses the system's local timezone, no explicit selector. If multi-timezone support is needed, follow-up trigger.
- **Auto-refresh on time range** (e.g., "last 1h" updating in real-time as time passes) — out of scope; presets compute `from = now - durationMs` once per click.
- **Backend additions for endpoints that don't exist** (Traces list, Agents list-by-time) — pages without time-ordered data don't need this filter.

## Risks

| Risk | Mitigation |
|---|---|
| Type-mismatch bug fix changes `from`/`to` from `number` to `string` in 4 frontend filter types — any consumer that today computes a numeric epoch and passes it as a filter value would silently break | The current behavior is already broken (numeric vs string string-comparison never matches); fixing the type is a forward step. PR description must explicitly call out the type change so consumers aware of the (broken) `number` shape see the migration. |
| Backend WHERE additions on `tasks.created_at` and `dev_runs` (which queries `tasks` table) might regress performance if `created_at` is not indexed | Verify `created_at` index status in db.rs schema (`CREATE INDEX` statements). If missing, file a follow-up index ticket; this PR adds the filter regardless. SQLite query planner uses `created_at` index when present and falls back to scan when not. |
| Tasks page gaining a top-level filter bar changes its layout | Body's AC requires "every list page exposes" the filter. Tasks is a list page (renders task rows); adding a filter bar matches expectation. PR description shows screenshots of the new bar. |
| `<TimeRangeFilter />` "Custom" picker uses native `<input type="datetime-local">` whose styling varies slightly across browsers | Native control is keyboard-accessible and provides date entry; visual fidelity to the Stitch template is the operator's call at PR review. If the variance is unacceptable, follow-up trigger to wrap in a styled component. |
| Concurrent edits to `packages/ui/CLAUDE.md` from mika#663/#657/#654/#655 (also seeding/extending the file) | Standard rebase-and-resolve at merge time. Each ticket's plan handles seed-or-extend. |
| Backend `from`/`to` ISO 8601 string compare relies on lexicographic ordering matching chronological ordering | True for any ISO 8601 string with consistent timezone (Z or +00:00). Plan emits Z-suffixed UTC ISO 8601 strings from `<TimeRangeFilter />` (`Date.toISOString()` produces this format by default). Backend stores timestamps as TEXT in the same format. Lexicographic ordering matches chronological. |
| If operator verifies Stitch screen at PR and finds significant visual divergence, rework needed | Plan is conservative on visual specifics — leans on the body's preset list and component design. If Stitch reveals a different shape (e.g., dropdown for presets instead of buttons), iteration after PR review is acceptable. The architectural shape (primitive + grammar + migration) holds regardless of visual polish. |
| 7-page migration is the largest dashboard touch in tonight's wave | Migration is mechanical per page (~5 lines each). PR splits naturally into commits per change so reviewer can land them incrementally. |

## Sequencing

**Atomic commit sequence (per architect Finding 5 — name the layered ordering, not just the changes):**

1. **Commit 1 — Change 1** (luminescent-core.md §5.4 grammar). Rulebook precedes code, same precedent as mika#657/#654/#655.
2. **Commit 2 — Change 3** (backend additions for 3 endpoints: Sessions, Tasks, DevRuns). Server-layer change independent of frontend; buildable alone.
3. **Commit 3 — Change 4** (type fixes: `from?: number` → `from?: string` in 4 existing files; add `from?: string`/`to?: string` to 3 new files). **Bug-fix commit — verifiable in isolation via curl** (see Change 4's named verification). Aligns frontend types to backend reality before any UI consumes them.
4. **Commit 4 — Change 2** (`<TimeRangeFilter />` library component). Library layer; depends on grammar (Commit 1) for the timezone assumption documentation.
5. **Commit 5 — Change 5** (migrate 7 dashboard pages). Consumer layer; depends on Commits 2-4 (component exists, types correct, backend supports filtering).
6. **Commit 6 — Change 6** (`packages/ui/CLAUDE.md` enforcement table — seed or extend). Documentation last.
7. **Pre-merge verification:**
   - `cargo check -p mika-agent`
   - `npm run build --prefix mika/packages/ui`
   - `npm run build --prefix mika/dashboard`
   - Manual curl: `/api/sessions?from=2026-04-26T20:00:00Z&to=2026-04-26T22:00:00Z` returns filtered subset (proves backend addition)
   - Manual curl: `/api/timeline?from=2026-04-26T00:00:00Z&to=2026-04-27T00:00:00Z` returns filtered subset (proves bug fix; was broken before with numeric input)
   - Dashboard visual: each migrated page renders the filter; click each preset, verify URL updates and result list shrinks
   - Timezone test (per Finding 1): set browser timezone to UTC+5; enter custom range; confirm emitted ISO strings are offset by +5h from input value
8. **Open PR** with:
   - Screenshots of each migrated page
   - Stitch reference for operator visual verification
   - Explicit before/after curl evidence for the bug fix

This commit sequence ensures every commit is buildable, the bug fix (Commit 3) ships before any consumer emits strings that depend on it, and reviewers can verify each layer independently. Without this ordering, partial implementations could land in an ambiguous correctness state.

## Verification

```bash
# Confirm rulebook extension
grep -c "5.4 Time-range filter affordance grammar" mika/docs/design/luminescent-core.md  # → 1
grep -c "Hand-rolling.*forbidden" mika/docs/design/luminescent-core.md  # → ≥ 4 (one per §5.x section after this PR)

# Confirm component exists + exports
test -f mika/packages/ui/src/components/TimeRangeFilter.tsx && echo "OK"
grep -c "TimeRangeFilter" mika/packages/ui/src/index.ts  # → 1

# Confirm backend filter additions (3 endpoints × 2 fields = 6 occurrences in dashboard.rs + dashboard_dev_runs.rs)
grep -c "from: Option<String>" mika/crates/mika-agent/src/server/dashboard.rs  # → ≥ 2 (Sessions + Tasks)
grep -c "from: Option<String>" mika/crates/mika-agent/src/server/dashboard_dev_runs.rs  # → 1 (DevRuns)
grep -c "started_at >= " mika/crates/mika-agent/src/db.rs  # → ≥ 2 (sessions + team-runs)
grep -c "created_at >= " mika/crates/mika-agent/src/db.rs  # → ≥ 4 (timeline + llm_calls + tool_calls + tasks/dev-runs)

# Four-command discovery sweep (per architect Finding 7 — completeness with expected output shapes)
# 1. Filter types use string, not number, for from/to (bug-fix verification)
grep -rn "from?: number\|to?: number" mika/dashboard/src/api/*.ts  # → 0 matches
# 2. All 7 pages import TimeRangeFilter from @senara-solutions/ui
grep -l "import.*TimeRangeFilter" mika/dashboard/src/pages/  # → exactly 7 files (Sessions, Timeline, LlmCalls, ToolCalls, Tasks, DevRuns, TeamRuns)
# 3. URL state for from/to is wired
grep -rn "updateFilter('from'" mika/dashboard/src/pages/*.tsx  # → ≥ 7 matches (one per migrated page)
# 4. Backend has WHERE created_at >= clauses across all 6 list endpoints (timeline + llm_calls + tool_calls + sessions + tasks + dev-runs); team-runs uses started_at
grep -c "created_at >= " mika/crates/mika-agent/src/db.rs  # → ≥ 5 occurrences (3 pre-existing + 2 added by Change 3 for sessions/tasks; dev-runs also adds one for tasks-table query)
grep -c "started_at >= " mika/crates/mika-agent/src/db.rs  # → ≥ 1 (team-runs, pre-existing)

# Manual timezone correctness test (per architect Finding 1)
# Set browser timezone to UTC+5; enter custom range 2026-04-26T00:00 to 2026-04-26T23:59;
# confirm emitted ISO strings are 2026-04-25T19:00:00Z and 2026-04-26T18:59:00Z (offset by -5h, since +5h browser local converts to -5h UTC offset).
# This proves the conversion is working, not silently truncating timezone data.

# Confirm CLAUDE.md enforcement
grep "TimeRangeFilter.*Audited clean.*mika#659" mika/packages/ui/CLAUDE.md  # → match

# Build verification
cargo check -p mika-agent
npm run build --prefix mika/packages/ui
npm run build --prefix mika/dashboard

# Manual integration test
curl -H "Authorization: Bearer $MIKA_DASHBOARD_TOKEN" "http://localhost:8080/api/sessions?from=2026-04-26T20:00:00Z&to=2026-04-26T22:00:00Z"  # → filtered subset
```

## Discovery items (verified during planning)

1. **4-of-8 endpoints already support `from`/`to`** at the SQL level (Timeline, LlmCalls, ToolCalls, TeamRuns); 3 need backend additions (Sessions, Tasks, DevRuns); 1 is detail-only (Traces). Plan handles all three layers.
2. **Latent type-mismatch bug** in 4 frontend filter types: `from?: number` vs backend ISO 8601 string compare. Current behavior is non-functional. Plan fixes by changing types to `string`.
3. **No date-picker component exists** in the codebase. Greenfield extraction. Native `<input type="datetime-local">` + button presets cover v1; no `date-fns` / `dayjs` dependency added.
4. **Stitch reference exists** (`c5b6feddb5444f3d83a7f9b94e140bcd`) — design has landed, unlike mika#658 (no Stitch session). Visual fidelity verified at PR review by operator (architect cannot fetch Stitch screens from grooming context).
5. **Tasks page has no current filter row** — adding a top-level filter bar is a slight UX shift but matches the body's "every list page exposes it" AC.
6. **Backend SQL pattern is uniform across endpoints** — every supported endpoint uses `<column> >= ? AND <column> <= ?` on a TEXT column with ISO 8601 storage. Pattern is mechanical for the 3 endpoints that need additions.
7. **`useSearchParamsFilter` integrates cleanly** — `<TimeRangeFilter />`'s `onChange({ from, to })` wires to two `updateFilter` calls (one per param). URL state is shareable per AC.
8. **Pre-commit discovery discipline applied** — three-command sweep in verification block (filter type strings, page imports, URL wiring) per the standing pattern.
