---
title: "feat(ui+dashboard): align StatusBadge/TaskStatusBadge to luminescent-core, add spacing tokens, migrate hand-rolled status pills"
type: feat
status: active
date: 2026-04-27
origin: senara-solutions/mika#657
---

# Plan — dashboard visual rhythm: tokens + status pill alignment + migration (mika#657)

**Issue:** [mika#657](https://github.com/senara-solutions/issues/657) — `Dashboard > Visual rhythm: recent-activity widgets and tables lack proportion, shared spacing tokens, consistent status pills`
**Branch:** `feat/657/dashboard-visual-rhythm-tokens-pills`
**Type:** feat (Phase 2 primitive in milestone #13)
**Labels:** enhancement, dashboard

## Problem (per issue body, leading callout)

> mostly subsumed by `mika/docs/design/luminescent-core.md` (rulebook now covers tokens, status pills, spacing, typography). Remaining scope: audit existing components against the rulebook and migrate any non-conforming instances.

The body's broader proposal (build out `<StatusPill />`, `<RecentActivityCard />`, table column ratio conventions, aspect ratios) is largely covered by sibling tickets. This plan focuses on the body's *actual* remaining scope.

## Audit results (verified during planning)

### What luminescent-core.md prescribes

| Concern | Rulebook prescription | Cite |
|---|---|---|
| Spacing scale | 8px base, doubled (`spacing-1`=4px, `spacing-2`=8px, `spacing-4`=16px, …, `spacing-16`=64px) | §6 |
| Color tokens | `--color-primary`, `--color-error`, `surface_*`, `on_surface_variant`, etc. — full table | §2 |
| Typography (status labels) | Use `label-md`/`label-sm` UPPERCASE with `0.05em` letter-spacing for "system metadata" | §3 |
| Active-agent chip | `surface_bright` bg + 2px `primary`-token glow dot | §5 |
| Roundedness | `xl` (1.5rem) or `lg` (1rem) for cards; pills can use `rounded-full` | §4 |
| Multi-state status grammar (success/warn/error/info/neutral) | **Silent** | — |
| Aspect ratios for cards | **Silent** | — |
| Row density tokens, "recent N" caps | **Silent** | — |
| ID truncation + copy | **Silent** | — |

The rulebook is **silent** on multi-state pill grammar, recent-N caps, and ID truncation. Those silences map to sibling tickets:
- Multi-state grammar → in scope here as a minimal extension, but only what's needed to migrate existing hand-rolled instances.
- Recent-N caps + aspect ratios → mika#658 (state catalog) or future Phase 4 cross-cutting work.
- ID truncation + copy → `<TraceIdWidget />` consumed by mika#651, mika#652, mika#653 (already named in those tickets' bodies).

### What `@senara-solutions/ui` ships today

| Component | File | API | Drift from rulebook |
|---|---|---|---|
| `<StatusBadge>` | `packages/ui/src/components/StatusBadge.tsx` | `{ active: boolean }` — binary only | Hardcoded emerald/amber, not design tokens; only 2 states |
| `<TaskStatusBadge>` | `packages/ui/src/components/TaskStatusBadge.tsx` | `{ status: string }` — 10 task states | Hardcoded yellow/blue/green/red/emerald/orange/gray/purple; labels not uppercase + tracking-wide |
| `<Pagination>` | `…/Pagination.tsx` | 4-prop API | OK (audited mika#663) |
| `<EmptyState>` | `…/EmptyState.tsx` | `{ message }` | Will be extended in mika#658 |
| `<CopyButton>` | `…/CopyButton.tsx` | `{ text, className?, title? }` | Visual-confirm refinement in mika#665 |
| `<MarkdownContent>` | `…/MarkdownContent.tsx` | `{ content }` | OK |

`packages/ui/src/theme.css` (13 lines, lines 1–13) defines color + font tokens but **no spacing tokens** — rulebook §6 prescribes `spacing-1` through `spacing-16` and they are absent.

### Hand-rolled status pills found in dashboard

Verified via `grep -rn "rounded-full" mika/dashboard/src/pages/*.tsx` plus targeted reads. Each row is a place where `<StatusBadge />` (after generalization) should be used:

| File | Line(s) | Current shape | Migrates to |
|---|---|---|---|
| `LlmCalls.tsx` | 130–147 | success/failed dot+text inline | `<StatusBadge variant="success" label="Success" />` / `variant="error" label="Failed"` |
| `LlmCallDetail.tsx` | 71–87 | same shape as LlmCalls | same |
| `ToolCallDetail.tsx` | 54–67 | success/failed dots inline | same |
| `TraceDetail.tsx` | 225–232 | success/failed dots inline | same |
| `Timeline.tsx` | 39 | live indicator with pulse | `<StatusBadge variant="success" label="Live" dotPulse />` |
| `SessionDetail.tsx` | various inline pills (209–372 region) | success/failed indicators | same |
| `ToolCalls.tsx` | 113–120 | success/failed dots inline | same |

**Out of scope (named explicitly):**
- `Sessions.tsx:137` — channel pill (`system` / `github`). This is a *content pill*, not a status pill. Different semantics. Files a follow-up ticket if needed; not migrated here.
- `ToolCalls.tsx:37–44` — `sourceBadge()` helper for tool source (builtin/skill/mcp). Also a typed-source classifier, not a status. Could be a `<SourceBadge />` primitive in a future ticket. Not migrated here.
- All UUID/trace-ID truncation drift — `<TraceIdWidget />` is sibling-ticket scope (#651/#652/#653).

## Approach

Five changes, two layers (`packages/ui/` + `dashboard/`):

### Change 1 — Add spacing tokens to `packages/ui/src/theme.css`

**File:** `mika/packages/ui/src/theme.css` (existing, 13 lines)

Per rulebook §6, add the 8px-base spacing scale to the `@theme` block. Tailwind v4 reads `--spacing-N` tokens from `@theme` and exposes them as utility classes (`p-1`, `gap-4`, etc., remapped to the token values).

```css
@theme {
  /* existing colors and fonts unchanged */

  /* Status pill variant for external-dependency wait (per Change 6 luminescent-core extension) */
  --color-blocked: #f97316;        /* orange-500 — sharper than warning, distinct at-a-glance */

  /* Spacing scale per luminescent-core §6 (8px base, doubled rhythm) */
  --spacing-1: 0.25rem;   /*  4px */
  --spacing-2: 0.5rem;    /*  8px */
  --spacing-3: 0.75rem;   /* 12px */
  --spacing-4: 1rem;      /* 16px */
  --spacing-5: 1.25rem;   /* 20px */
  --spacing-6: 1.5rem;    /* 24px */
  --spacing-8: 2rem;      /* 32px */
  --spacing-12: 3rem;     /* 48px */
  --spacing-16: 4rem;     /* 64px */
}
```

Note: Tailwind v4's default spacing scale is already 4px-based. This change makes the scale **explicit** in the design system layer, establishing tokens future components can reference by name (e.g., a row-density component can declare `padding: var(--spacing-4)` instead of `p-4`). For *existing* components, no migration is required — Tailwind utilities continue to work because the resolved values are identical to defaults.

**Why this matters even though values match Tailwind defaults:** the rulebook prescribes the scale; making it explicit in `@theme` (a) names the intent (these are design tokens, not Tailwind defaults that happen to be 4px), (b) lets `<StatusBadge />` and future components reference `var(--spacing-N)` directly when CSS-in-JS or computed styling is needed, and (c) makes drift visible — anyone changing the scale changes the rulebook, not a coincidence. Net diff: ~12 lines added.

### Change 2 — Generalize `<StatusBadge />` from binary to multi-variant (six variants)

**File:** `mika/packages/ui/src/components/StatusBadge.tsx` (existing, 22 lines)

Current API: `{ active: boolean }`. Replace with:

```typescript
interface StatusBadgeProps {
  variant: 'success' | 'warning' | 'error' | 'info' | 'neutral' | 'blocked'
  label: string
  dotPulse?: boolean
}
```

**Variant count resolved at plan time (per architect Finding 2, first-pass):** six variants, not five. The sixth variant `blocked` is included because external-dependency-wait carries meaningfully distinct semantics from "not yet started" (`pending` → `warning`) and "paused, resumable" (`suspended` → `warning`). At-a-glance visual distinction matters — collapsing three task states (`pending`, `blocked`, `suspended`) into one `warning` variant loses signal. `blocked` gets its own variant; `pending` and `suspended` map to `warning` (text discriminates).

Variants map to design-token-derived classNames (using existing `--color-success`, `--color-warning`, `--color-error`, `--color-accent`, `--color-muted`, plus a new `--color-blocked` derived from rulebook `--color-warning` with sharper saturation):

| variant | bg | text | dot | semantic meaning |
|---|---|---|---|---|
| `success` | `bg-success/10` | `text-success` | `bg-success` | Operation completed successfully; agent active |
| `warning` | `bg-warning/10` | `text-warning` | `bg-warning` | Degraded, paused, or pending — caution but not failure |
| `error` | `bg-error/10` | `text-error` | `bg-error` | Operation failed; needs intervention |
| `info` | `bg-accent/10` | `text-accent` | `bg-accent` | Active operation in progress; informational state |
| `neutral` | `bg-white/[0.06]` | `text-muted` | `bg-muted` | Cancelled, archived, or no-state |
| `blocked` | `bg-blocked/15` | `text-blocked` | `bg-blocked` | External-dependency wait; distinct from warning to preserve at-a-glance signal |

Labels render UPPERCASE with `tracking-wide` (Tailwind utility for `0.05em` letter-spacing), per rulebook §3:

```tsx
<span className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase tracking-wide ${variantBg} ${variantText}`}>
  <span className={`w-1.5 h-1.5 rounded-full ${variantDot} ${dotPulse ? 'animate-pulse' : ''}`} />
  {label}
</span>
```

**Migrating existing binary callsites:**
- `Agents.tsx` (line 46 import + render) → `<StatusBadge variant={agent.active ? 'success' : 'warning'} label={agent.active ? 'Active' : 'Inactive'} />`
- `AgentDetail.tsx` (line 93 import + render) → same shape

**Net diff (Change 2):** ~30 lines in `StatusBadge.tsx` (rewrite component), ~2 lines per existing callsite (2 callsites = 4 lines).

### Change 3 — Refactor `<TaskStatusBadge />` to thin adapter delegating to `<StatusBadge />`

**File:** `mika/packages/ui/src/components/TaskStatusBadge.tsx` (existing, 27 lines)

Per architect Finding 3 (first-pass): `<TaskStatusBadge />` is **NOT merged into `<StatusBadge />`**; it becomes a thin adapter. Typed `status: string` (the task vocabulary) → `variant + label` mapping lives in `<TaskStatusBadge />`; the actual rendering primitive is `<StatusBadge />`. This factoring (a) keeps a typed domain API for task consumers (no re-derivation of `task → variant` at every callsite), (b) ensures consistent rendering by routing through a single primitive, and (c) prevents future refactor pressure to merge — the delegation pattern is the architecture.

```typescript
import StatusBadge from './StatusBadge'

const TASK_VARIANT_MAP: Record<string, { variant: 'success' | 'warning' | 'error' | 'info' | 'neutral' | 'blocked'; label?: string }> = {
  pending: { variant: 'warning', label: 'PENDING' },
  in_progress: { variant: 'info', label: 'IN PROGRESS' },
  running: { variant: 'info', label: 'RUNNING' },
  completed: { variant: 'success', label: 'COMPLETED' },
  delivered: { variant: 'success', label: 'DELIVERED' },
  failed: { variant: 'error', label: 'FAILED' },
  blocked: { variant: 'blocked', label: 'BLOCKED' },
  cancelled: { variant: 'neutral', label: 'CANCELLED' },
  suspended: { variant: 'warning', label: 'SUSPENDED' },
  recurring_active: { variant: 'info', label: 'RECURRING' },
}

const DEFAULT = { variant: 'neutral' as const, label: undefined }

export default function TaskStatusBadge({ status }: { status: string }) {
  const mapped = TASK_VARIANT_MAP[status] ?? DEFAULT
  return <StatusBadge variant={mapped.variant} label={mapped.label ?? status.replace(/_/g, ' ').toUpperCase()} />
}
```

**Six-variant mapping (resolved at plan time per architect Finding 2):**
- `pending`, `suspended` → `warning` (caution, not failure)
- `in_progress`, `running`, `recurring_active` → `info` (active operation)
- `completed`, `delivered` → `success`
- `failed` → `error`
- `blocked` → `blocked` (sixth variant — external wait, visually distinct from warning)
- `cancelled` → `neutral`

`recurring_active` maps to `info` rather than its current `purple` because `purple` is not in the rulebook palette and the semantics ("scheduled-recurring, currently active") fits `info`. If Vincent's hand-rolled-purple was load-bearing, raise as a follow-up — purple is not a luminescent-core token.

API stays `{ status: string }` — no consumer changes. Net diff: full rewrite from inline `STATUS_STYLES` map (with hardcoded Tailwind classes) to the adapter shape (~25-line file replacing 27 lines).

### Change 4 — Migrate hand-rolled status pills to `<StatusBadge />`

**Files (7):** `dashboard/src/pages/{LlmCalls,LlmCallDetail,ToolCalls,ToolCallDetail,TraceDetail,Timeline,SessionDetail}.tsx`

For each hand-rolled success/failed/live pill listed in the audit table, replace the inline `<span>` with `<StatusBadge variant="…" label="…" />`. The audit table above names every callsite by file:line. Net diff: ~7 lines deleted + ~3 lines added per callsite × ~10 callsites = ~70 lines net reduction (component substitution shrinks each callsite).

After migration, `grep -rn "inline-flex items-center.*rounded-full" mika/dashboard/src/pages/` should return only:
- `Sessions.tsx:137` (channel pill — out of scope)
- `ToolCalls.tsx:113–120` if not migrated (success/failed) — verify migrated
- Any pills inside `packages/ui/` components (StatusBadge.tsx, TaskStatusBadge.tsx, etc. — bubbles up)

This grep becomes the post-migration drift detector (and is added to the verification block).

### Change 6 — Extend `luminescent-core.md` with multi-state status pill grammar

**File:** `mika/docs/design/luminescent-core.md` (existing rulebook)

**Per architect Finding 1 (first-pass, dispatch-blocker):** the rulebook is silent on multi-state grammar (success/warning/error/info/neutral/blocked) but Change 2 introduces a six-variant `<StatusBadge />` API. Without naming this as a rulebook extension, the codebase silently diverges from the design system doc — next designer or developer reads luminescent-core.md, doesn't see multi-state grammar, and either re-derives differently or treats `<StatusBadge />`'s variants as implementation detail rather than design system contract.

Add a new subsection to luminescent-core.md (likely under §5 "AI Agent Status Chips" or as a new §5.1):

```markdown
### 5.1 Multi-state status grammar

The active-agent chip (§5) is the canonical surface form. For surfaces requiring multi-state status indication (success/failed operations, pending/blocked task states, info/neutral classifications), the design system declares six variants. `<StatusBadge />` from `@senara-solutions/ui` is the canonical rendering primitive for this grammar.

| Variant | Token | Semantic meaning |
|---|---|---|
| `success` | `--color-success` | Operation completed successfully; agent active; positive terminal state |
| `warning` | `--color-warning` | Degraded, paused, pending — caution but not failure; can resume |
| `error` | `--color-error` | Operation failed; intervention required |
| `info` | `--color-accent` | Active operation in progress; informational/in-motion state |
| `neutral` | `--color-muted` | Cancelled, archived, or stateless; no active signal |
| `blocked` | `--color-blocked` | External-dependency wait; visually distinct from `warning` to preserve at-a-glance signal when paired with pending/suspended states in tabular contexts |

Labels render UPPERCASE with `tracking-wide` (`0.05em` letter-spacing) per §3 typography. The active-agent chip (§5) is a specialized form of `success` with the `dotPulse` modifier.

**Hand-rolled status pills are forbidden.** Any new surface code rendering its own status pill (success/error inline indicators, custom colored dots with text) is a review fail. Use `<StatusBadge variant="..." label="..." />` from `@senara-solutions/ui`. For task-domain status (`pending`, `in_progress`, `completed`, etc.), use `<TaskStatusBadge status={...} />` which delegates to `<StatusBadge />` with the canonical task→variant mapping.
```

This addition is the design-system contract for the grammar Change 2 introduces. It declares variants, semantic meaning, canonical rendering surface, and the hand-rolled-forbidden rule. Future variants extend the table here, not in the codebase first.

Net diff: ~25 lines added to `luminescent-core.md`.

### Change 5 — Document canonical-primitive enforcement in `packages/ui/CLAUDE.md`

**File:** `mika/packages/ui/CLAUDE.md` (does not exist yet — `mika#663` plan creates it)

If `mika#663` ships first, extend its enforcement table with a `<StatusBadge />` row (Audited clean — mika#657). If `mika#657` ships first, this plan creates `packages/ui/CLAUDE.md` from scratch using the same shape as `mika#663`'s plan (component table + escape-hatch + migration-status column) and seeds it with `<StatusBadge />` and `<TaskStatusBadge />` as `Audited clean (mika#657)`.

Sequencing note: ship-order is operator's call. Either ticket can seed the file; the other extends it. The plan files for both tickets reference `packages/ui/CLAUDE.md` as a load-bearing artifact, so whichever ships second updates the existing table.

**Net diff:** if seeding the file, ~50 lines (per `mika#663`'s plan). If extending, ~5 lines (one row + entry in narrative).

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `mika/packages/ui/src/theme.css` | +13 lines: `--color-blocked` + `--spacing-1` through `--spacing-16` in `@theme` block |
| 2 | `mika/packages/ui/src/components/StatusBadge.tsx` | Full rewrite: ~22 lines → ~32 lines, API change `{active}` → `{variant, label, dotPulse?}`, six variants |
| 2 | `mika/dashboard/src/pages/Agents.tsx` | Update binary callsite → variant API |
| 2 | `mika/dashboard/src/pages/AgentDetail.tsx` | Update binary callsite → variant API |
| 3 | `mika/packages/ui/src/components/TaskStatusBadge.tsx` | Refactor to thin adapter delegating to `<StatusBadge />`; typed task→variant map |
| 4 | `mika/dashboard/src/pages/LlmCalls.tsx` | Replace inline success/failed pills with `<StatusBadge>` |
| 4 | `mika/dashboard/src/pages/LlmCallDetail.tsx` | Same |
| 4 | `mika/dashboard/src/pages/ToolCalls.tsx` | Same (success/failed only — sourceBadge stays out of scope) |
| 4 | `mika/dashboard/src/pages/ToolCallDetail.tsx` | Same |
| 4 | `mika/dashboard/src/pages/TraceDetail.tsx` | Same |
| 4 | `mika/dashboard/src/pages/Timeline.tsx` | Replace Live indicator with `<StatusBadge variant="success" label="Live" dotPulse />` |
| 4 | `mika/dashboard/src/pages/SessionDetail.tsx` | Replace inline success/failed pills (multiple in 209–372 region) |
| 5 | `mika/packages/ui/CLAUDE.md` | Add `<StatusBadge />` row to enforcement table (or seed file if not yet created) |
| 6 | `mika/docs/design/luminescent-core.md` | +25 lines: new §5.1 multi-state status pill grammar (six variants, semantic meanings, canonical rendering surface, hand-rolled-forbidden rule) |

Estimated diff: ~150-200 lines across 12-13 files. Net reduction in dashboard/ since substitutions are shorter than originals; net addition in packages/ui/.

## Tests

`@senara-solutions/ui` currently has no test scaffolding (verified — no `packages/ui/**/*.test.{ts,tsx}` files exist). Verification is by:

1. **Build verification** — `npm run build` in `packages/ui/` and `dashboard/` both succeed without TypeScript errors after API changes.
2. **Visual verification** — `npm run dev:dashboard` (per root CLAUDE.md), navigate to each migrated page (Agents, LlmCalls, ToolCalls, Timeline, etc.) and verify status pills render with token colors, uppercase labels, tracking-wide letter-spacing, and the Live indicator on Timeline pulses.
3. **Drift grep** — `grep -rn "inline-flex items-center.*rounded-full" mika/dashboard/src/pages/` should return only `Sessions.tsx:137` (channel — out of scope) plus pills inside library components. Any other match means a hand-rolled instance was missed.
4. **Color-token grep** — `grep -rn "bg-emerald-400\|bg-red-400\|bg-amber-400" mika/packages/ui/src/components/` should return zero matches after Change 2 + Change 3 land. All colors must reference design tokens.

If/when `@senara-solutions/ui` test scaffolding is added (separate ticket), `<StatusBadge />` and `<TaskStatusBadge />` should get unit tests for variant→className mapping. Out of scope here.

## Acceptance criteria

- [ ] `mika/docs/design/luminescent-core.md` includes a new §5.1 declaring the six-variant multi-state grammar (success/warning/error/info/neutral/blocked) with semantic meanings, canonical rendering surface (`<StatusBadge />`), and the hand-rolled-forbidden rule.
- [ ] `mika/packages/ui/src/theme.css` declares `--color-blocked` and `--spacing-1` through `--spacing-16`.
- [ ] `mika/packages/ui/src/components/StatusBadge.tsx` API is `{ variant, label, dotPulse? }` with six variants; binary `active: boolean` API removed.
- [ ] `mika/packages/ui/src/components/TaskStatusBadge.tsx` is refactored to a thin adapter that delegates to `<StatusBadge />` via a typed task→variant mapping. No standalone rendering remains.
- [ ] `<StatusBadge />` (and therefore `<TaskStatusBadge />` via delegation) renders labels UPPERCASE with `tracking-wide`.
- [ ] All hand-rolled success/failed/live pills in `dashboard/src/pages/{LlmCalls,LlmCallDetail,ToolCalls,ToolCallDetail,TraceDetail,Timeline,SessionDetail}.tsx` use `<StatusBadge />` (per audit table file:line).
- [ ] `grep -rn "bg-emerald-400\|bg-red-400\|bg-amber-400" mika/packages/ui/src/components/` returns zero matches.
- [ ] Secondary sweep: `grep -rn "bg-.*green\|bg-.*red\|bg-.*emerald\|bg-.*amber\|text-emerald-\|text-red-\|text-yellow-" mika/dashboard/src/pages/*.tsx` matches only inside out-of-scope items (channel pill, source badge, lucide icon containers); each match is named in the PR description as out-of-scope.
- [ ] `grep -rn "inline-flex items-center.*rounded-full" mika/dashboard/src/pages/` returns only out-of-scope rows.
- [ ] `mika/packages/ui/CLAUDE.md` enforcement table lists `<StatusBadge />` with `Audited clean (mika#657)` migration status. (If file doesn't yet exist, this plan seeds it; if `mika#663` shipped first, extends it.)
- [ ] `npm run build` succeeds in `packages/ui/` and `dashboard/`.
- [ ] Visual check on migrated pages (success/failed/live indicators + task statuses) confirms token colors render correctly and `blocked` is visually distinct from `warning`.

## Out of scope

- **Channel pill in `Sessions.tsx`** — content pill, not status. File a `<ChannelPill />` ticket if drift surfaces.
- **`sourceBadge()` helper in `ToolCalls.tsx`** — typed-source classifier (builtin/skill/mcp), not status. File a `<SourceBadge />` ticket if migration is justified.
- **`<TraceIdWidget />`** — explicitly named in mika#651/#652/#653 issue bodies; ID truncation + copy belongs there.
- **`<RecentActivityCard />`** — issue body's broader proposal; rulebook is silent on row caps and aspect ratios; defer to Phase 4 cross-cutting work or fold into mika#658's state-catalog scope.
- **Aspect-ratio convention for cards** — rulebook silent; Stitch design pass would own this.
- **Table column ratio rules** — rulebook silent; Stitch design pass would own this.
- **`<EmptyState />` extensions to `<LoadingState />`/`<ErrorState />`** — mika#658's scope.
- **`<CopyButton />` visual-confirm refinement** — mika#665's scope (already groomed).
- **Adding new variants beyond `success/warning/error/info/neutral`** to `<StatusBadge />` — only the five variants the rulebook supports cleanly, or a sixth `blocked` if `<TaskStatusBadge />` collapse loses signal (decided in implementation).
- **Removing or redesigning `<TaskStatusBadge />`** — keep as task-domain specialization; only realign colors and label styling. Renaming it to `<StatusPill>` or merging into `<StatusBadge />` would break consumers and isn't justified by the body's "audit-and-migrate" framing.
- **Schema for `tracking-wide`** — Tailwind v4 utility maps to `0.05em` per default; matches rulebook §3 prescription. No theme.css change needed.

## Risks

| Risk | Mitigation |
|---|---|
| Generalizing `<StatusBadge />` from `{active}` to `{variant, label}` breaks all 2 binary callsites | Both callsites (`Agents.tsx`, `AgentDetail.tsx`) are migrated in Change 2 atomically. TypeScript catches any missed callsite at build time. |
| `<TaskStatusBadge />` color realignment loses meaningful distinction (e.g., blocked vs pending both → warning) | Implementation evaluates side-by-side; if collapse loses signal, add `blocked` as a sixth variant to `<StatusBadge />` in this PR. Explicit fallback path. |
| Hand-rolled pill substitution renders differently because the original used different sizing | Verify by reading the original className for each migration target — most use `text-xs` or `text-[10px]`; `<StatusBadge />`'s `text-[10px] font-semibold` matches the dominant pattern. Outliers documented in PR. |
| Sequencing collision with mika#663 on `packages/ui/CLAUDE.md` | Both plans reference the same file. Ship-order is operator's call; whichever ships second extends rather than seeds. Both plans explicitly handle either case in Change 5. |
| `--spacing-N` tokens conflict with Tailwind v4's default scale | Verified — Tailwind v4 reads `--spacing-N` from `@theme` as overrides; if values match defaults, behavior is identical. The intent is explicitness; existing `p-4`, `gap-2`, etc. continue to work. |
| Visual check requires running the dashboard locally — easy to skip | AC explicitly names visual check; PR description must include screenshots of migrated pages. Reviewer fails the AC if screenshots are absent or pills look wrong. |

## Sequencing

1. **Change 6 first** (luminescent-core.md grammar extension). Rulebook declares the six-variant grammar before code consumes it. Architectural ordering: design system contract precedes implementation.
2. **Change 1 second** (theme.css `--color-blocked` + spacing tokens). Adds the design tokens Change 6 references.
3. **Change 2 third** (StatusBadge generalization + 2 callsite migrations). Implements the grammar declared in Change 6 using tokens from Change 1.
4. **Change 3 fourth** (TaskStatusBadge thin-adapter refactor). Delegates to Change 2's `<StatusBadge />`.
5. **Change 4 fifth** (migrate hand-rolled status pills across 7 dashboard pages). Depends on Change 2 (new `<StatusBadge />` API).
6. **Change 5 last** (`packages/ui/CLAUDE.md` enforcement table — seed or extend depending on mika#663's ship state).
7. **Visual + drift verification** (run greps, run dashboard, screenshot migrated pages).
8. **Open PR** cross-referencing #657 with screenshots.

Note on PR ordering: Changes 1, 2, 6 are tightly coupled (rulebook + tokens + component) and must ship in the same PR. Changes 3, 4, 5 build on that foundation but are not order-sensitive within the PR — a reviewer can read them in any order. Changes 1+2+6 are reviewed first.

## Verification

```bash
# Confirm rulebook extension declares the multi-state grammar
grep -c "5.1 Multi-state status grammar" mika/docs/design/luminescent-core.md  # → 1
grep -c "Hand-rolled status pills are forbidden" mika/docs/design/luminescent-core.md  # → 1

# Confirm spacing tokens + --color-blocked declared
grep -c "^\s*--spacing-" mika/packages/ui/src/theme.css  # → 9 (one per spacing-1, -2, -3, -4, -5, -6, -8, -12, -16)
grep -c "^\s*--color-blocked" mika/packages/ui/src/theme.css  # → 1

# Confirm StatusBadge six-variant API
grep -A 5 "interface StatusBadgeProps" mika/packages/ui/src/components/StatusBadge.tsx
# → expects variant: 'success' | 'warning' | 'error' | 'info' | 'neutral' | 'blocked'

# Confirm TaskStatusBadge delegates to StatusBadge (not standalone rendering)
grep -c "import StatusBadge" mika/packages/ui/src/components/TaskStatusBadge.tsx  # → 1
grep -c "<StatusBadge" mika/packages/ui/src/components/TaskStatusBadge.tsx  # → 1

# Confirm no hardcoded Tailwind colors in library components (primary drift detector)
grep -rn "bg-emerald-400\|bg-red-400\|bg-amber-400\|bg-yellow-400\|bg-blue-400\|bg-purple-400\|bg-orange-400\|bg-gray-400" mika/packages/ui/src/components/  # → 0 matches

# Confirm no hardcoded Tailwind status colors in dashboard pages (per architect aux finding — secondary sweep for color-hardcoded patterns the structural grep might miss)
grep -rn "bg-.*green\|bg-.*red\|bg-.*emerald\|bg-.*amber\|text-emerald-\|text-red-\|text-yellow-" mika/dashboard/src/pages/*.tsx  # → matches only inside out-of-scope items (channel pill, source badge, lucide icon containers)

# Confirm hand-rolled status pills migrated
grep -rn "inline-flex items-center.*rounded-full" mika/dashboard/src/pages/ | grep -v "Sessions.tsx:137"  # → ideally empty (ToolCalls source badge if still hand-rolled is expected and named in PR description)

# Confirm packages/ui/CLAUDE.md lists StatusBadge as audited clean
grep "StatusBadge.*Audited clean.*mika#657" mika/packages/ui/CLAUDE.md  # → match

# Build verification
npm run build --prefix mika/packages/ui  # → succeeds
npm run build --prefix mika/dashboard    # → succeeds
```

## Discovery items (verified during planning)

1. **luminescent-core.md is silent on multi-state status grammar** — only specifies the active-agent chip (§5). The five-variant `<StatusBadge />` shape (success/warning/error/info/neutral) is a minimal extension justified by the migration targets, not by the rulebook directly. Worth flagging to architect.
2. **`packages/ui/src/theme.css` lacks spacing tokens entirely** — only color and font tokens. Rulebook §6 explicitly prescribes the 8px-doubled scale.
3. **`<StatusBadge />` is currently binary** — `{ active: boolean }`. Generalizing to multi-variant is the migration's foundational change; without it, hand-rolled pills can't migrate to `<StatusBadge />`.
4. **`<TaskStatusBadge />` has 10 states with arbitrary Tailwind colors** — domain vocabulary is correct; color mapping needs realignment to design tokens.
5. **Hand-rolled status pills found in 7 dashboard files** — all success/failed/live patterns; verified by grep. Channel pills and tool source badges are explicitly different concerns and out of scope.
6. **`packages/ui/CLAUDE.md` is shared-artifact territory with mika#663** — both plans reference it. Ship-order matters but is operator's call; both plans handle either case.
7. **No test scaffolding in `packages/ui/`** — verification is by build + visual check + drift grep. Future ticket can add Vitest/RTL scaffolding.
