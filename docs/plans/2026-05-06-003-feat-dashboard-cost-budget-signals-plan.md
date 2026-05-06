---
ticket: mika#667
type: feat
title: Dashboard cost / budget signals — threshold warnings and cost visibility across surfaces
date: 2026-05-06
seq: 003
---

# Plan: dashboard cost / budget signals (mika#667)

## Why

Autonomous runs burn real money per LLM call. Today's observed costs are bounded by luck, not by system: mika#648 was \$12, mika#608's aborted run was \$27, and a poorly-scoped ticket or stuck loop could easily run \$200+ before anyone notices. The cost data exists in the database (the agent already records `cost_usd` on dev runs and individual LLM calls), but it is not surfaced anywhere an operator habitually looks. Today an operator must drill into a run's metadata blob or query the DB directly to see what something cost.

The fix bar is to make cost a **first-class signal** in the dashboard — visible wherever a run, call, session, or agent is shown, aggregated on the landing page, and primitive-backed so all consumers display it identically. The ticket's design landed in Stitch on 2026-05-05 with explicit screens for the `<CostMeter>` primitive, its Dev Run detail integration, and its landing-widget placement. This plan executes that design.

This ticket is part of milestone#13 (Dashboard observability). Layer 3 (engine-side per-run cost ceiling enforcement) is **explicitly deferred** — sibling ticket in milestone#8 if/when the visibility surface confirms the need.

## Phase 0 — Pin (verified state, source-anchored)

All paths verified against the worktree at `feat/667/dashboard-cost-budget-signals` HEAD `48e52c83` (main).

### Existing primitives

- **`packages/ui/src/components/TokenBudgetBar.tsx`** — three-tier color threshold progress bar with ARIA meter semantics. Per `mika/CLAUDE.md`: "TokenBudgetBar (three-tier color threshold progress bar with ARIA meter semantics)". Domain: token usage vs budget (e.g., context window, daily token limit). **Not the right primitive for cost** — token domain, fixed-cap semantics, ARIA `meter` role implies a known maximum. Cost has no fixed cap; thresholds are configurable warning/critical levels, not "100% full." Phase 1 establishes `<CostMeter>` as a sibling primitive, not a TokenBudgetBar wrapper.
- **`dashboard/src/components/CostTrendChart.tsx`** — time-series cost chart. Shipped via PR #976 / mika#660 (merged 2026-05-05). Currently used at `dashboard/src/pages/LlmCalls.tsx:118` driven by `useCostTrend(costTrendFilters)` hook. Lives in `dashboard/` not `packages/ui/` — single-consumer component. Phase 3's landing widget will import directly from dashboard/components, no need to publish to packages/ui (publishing would be premature per `feedback_keep_simple.md`).
- **`packages/ui/src/components/StatusBadge.tsx`** + **`ListRow.tsx`** — used as composition primitives for the LLM Calls list cost-column variant.

### Cost data API surface

- **`crates/mika-agent/src/server/dashboard_dev_runs.rs:36`** — `cost_usd: Option<f64>` field on the dev runs detail/list response. Already plumbed end-to-end (DB → Rust → JSON → React). No backend work needed for Phase 2's Dev Run detail integration.
- **`crates/mika-agent/src/server/dashboard_dev_runs.rs:44, 63, 90`** — `cost_usd` is read from the underlying tasks/checkpoints rows and surfaced via the API. The pin verifies it's a present, non-derived field at the API layer.
- **`dashboard/src/api/cost-trend.ts`** (assumed; confirm at implementation) — `useCostTrend` hook used by LLM Calls page. Returns `{ buckets, bucket_size, has_estimated_pricing }`. Phase 3 reuses this hook unmodified.
- **LLM Calls list cost** — needs verification. The pin found `useCostTrend` for the chart but the **list cost column** is a separate concern. If the LLM calls list endpoint doesn't already expose per-call cost, that's a minor backend addition (Phase 2.B). Verify at implementation.
- **Agent detail today's-cost-chip** — needs verification. The pin found no `cost` references in `dashboard/src/pages/AgentDetail.tsx` — this is a green-field integration. The data exists at the DB level (sum of LLM call costs filtered by agent + 24h window); whether there's an existing API endpoint or whether one needs adding is a Phase 0.5 verification step.

### Stitch design source of truth

Per the ticket body callouts, three Stitch screens anchor the design:
- `d0a5de04122147ffaa12879434db83cf` — `<CostMeter>` component sheet (the primitive's design)
- `f8f570be528e4f589b6dc56bd01f187b` — Dev Run detail integration (Layer 1 surface)
- `2443ccdf05d44295ade2f7c17574d7c3` — Landing widget placement (Layer 2 surface, depends on mika#666)

These are the canonical source for visual decisions. The plan does not relitigate design choices that the screens have settled — it executes them. If the implementer hits a question the screens don't answer (e.g., "what's the threshold color when cost < \$5?"), surface it as an issue for Vincent rather than guessing.

### Stitch project ID
- `6562713725762717689` — Mika Observability Dashboard project. Reference for cross-screen consistency checks.

### Dependencies on in-flight work

- **mika#666** — Dashboard landing page. Currently being implemented by the autonomous loop (claude-pilot active on `feat/666/dashboard-landing-home-overview` worktree as of plan-write time). Phase 3's landing widget MUST coordinate with whatever widget-slot interface mika#666 establishes. If mika#666 ships before this PR, Phase 3 reads its widget contract; if it ships during this PR, Phase 3 may need a one-line revisit to align imports.
- **mika#660 / PR #976** — `<CostTrendChart>`. Already merged. No risk of drift.

## Scope

**In scope (Layer 1 + Layer 2 from the ticket):**

- Phase 1: `<CostMeter>` primitive added to `packages/ui/src/components/`. ARIA-correct, three-tier threshold colors per Stitch sheet, two size variants (full, compact-chip).
- Phase 2: Per-surface integration on Dev Run detail (full meter), LLM Calls list (compact-chip cost column), Agent detail (today's-cost chip).
- Phase 3: Cost section on the home overview landing page (mika#666), reusing `<CostTrendChart>` directly.
- Phase 4: Tests (unit for the primitive, integration on each consumer page, visual regression via existing Storybook patterns), docs (`packages/ui/CLAUDE.md` enumeration update + design system reference).

**Out of scope (explicitly):**

- **Layer 3 — engine-side per-run cost ceiling enforcement.** A separate sibling ticket in milestone#8 (Agent reliability) when the visibility surface confirms operator demand for it. The plan's Phase 5 files this as a follow-up note, not as inlined code.
- **A dedicated `/dashboard/costs` page.** The ticket proposes "either a dedicated page OR a widget on home." Per the Stitch design (screen `2443ccdf05d44295ade2f7c17574d7c3`), the widget-on-home approach was selected. The dedicated page is not in scope. A future ticket can elevate the widget to a page if richer cost analytics are needed.
- **Anomaly detection ("this run is 3× the median for similar tickets").** The ticket lists this in the proposed surface area, but the Stitch design does not include anomaly badges. Defer to a follow-up if operator feedback requests it.
- **Cost ceiling configuration UI** — depends on Layer 3 existing.
- **Migration of `<CostTrendChart>` from `dashboard/components/` to `packages/ui/`.** Not needed; single consumer remains the dashboard. Promote-on-demand.

**Position on TokenBudgetBar reuse vs new primitive:**

`<CostMeter>` is a **new primitive**, not a styled `<TokenBudgetBar>` wrapper. Reasoning:

1. **Domain semantics differ.** TokenBudgetBar is "X of Y tokens used" — bounded, ARIA `meter` role implies a known maximum. CostMeter is "X dollars spent, configurable warning/critical thresholds" — unbounded, threshold-based. Different ARIA pattern (`status` + threshold via aria-valuenow + aria-valuemin/max with a dynamic max), different semantics for screen readers.
2. **Visual treatment may differ per Stitch sheet.** The component sheet `d0a5de04122147ffaa12879434db83cf` shows the design; if it diverges from TokenBudgetBar, hand-rolling a wrapper around TokenBudgetBar would either constrain the design or fight the wrapper. New primitive is cleaner.
3. **Future divergence is likely.** Cost meters may evolve (anomaly badges, trend sparklines, per-model breakdown tooltips) that are unrelated to token meters. Keeping them separate avoids `TokenBudgetBar` becoming a generic "any-meter."
4. **`packages/ui/CLAUDE.md` policy** — primitives are added when there's a real shared use case. CostMeter has three consumers in this PR alone (Dev Run detail, LLM Calls list, Agent detail) plus the future landing-widget reuse. Threshold for promotion is met on its own merits.

## Phase 1 — `<CostMeter>` primitive

**Files added:**
- `packages/ui/src/components/CostMeter.tsx` — implementation
- `packages/ui/src/components/CostMeter.test.tsx` — unit tests

**Files updated:**
- `packages/ui/src/index.ts` — re-export
- `packages/ui/CLAUDE.md` — primitive enumeration

**Component contract:**

```tsx
type CostMeterVariant = 'full' | 'chip'

interface CostMeterProps {
  /** Current cost in USD, expected non-negative. NaN/null renders empty state. */
  value: number | null
  /** Warning threshold in USD. Cost ≥ warning → amber. */
  warningAt?: number
  /** Critical threshold in USD. Cost ≥ critical → red. */
  criticalAt?: number
  /** 'full' (label + value + threshold bar) | 'chip' (compact inline). Default: 'full'. */
  variant?: CostMeterVariant
  /** Optional label override (default: 'Cost'). */
  label?: string
  /** ARIA description override for screen readers. */
  ariaLabel?: string
}
```

**Threshold semantics:**
- value < warningAt → green/neutral (default state)
- warningAt ≤ value < criticalAt → amber/warning
- value ≥ criticalAt → red/critical
- Both thresholds optional. If neither supplied, neutral always.
- Default thresholds (configurable per consumer): warning at \$5, critical at \$20. Confirm against Stitch sheet.

**ARIA pattern:**
- Role: `status` (not `meter` — meter requires a known maximum which cost doesn't have)
- `aria-live`: `polite` for the chip variant on inflight surfaces (cost updates as a run progresses)
- `aria-label`: composed from `label` + formatted value + threshold state ("Cost \$12.34, warning")

**Visual treatment:**
- Defer to Stitch screen `d0a5de04122147ffaa12879434db83cf`. The implementer reads the screen and matches it. Color tokens come from the existing design system (`packages/ui/src/theme.css`).

**Tests (CostMeter.test.tsx):**
- Renders empty state when value is null/NaN
- Renders neutral when value < warningAt (or no thresholds)
- Renders warning state at warningAt (boundary)
- Renders critical state at criticalAt (boundary)
- Chip variant has ARIA-live polite
- Full variant has correct label + value + threshold bar elements
- Custom label override flows through

## Phase 2 — Per-surface integration (Layer 1)

### Phase 2.A — Dev Run detail full meter

**File:** `dashboard/src/pages/DevRunDetail.tsx` (verify exact path at implementation; the ticket says "Dev Run detail" — it could be `DevRunDetail.tsx` or `DevRuns/[id].tsx`).

**Change:**
- Import `<CostMeter>` from `@senara-solutions/ui`
- Render in the run's metadata header section (top of detail page), variant `full`
- Pass `value={run.cost_usd}` (the existing API field per Phase 0 pin)
- Thresholds: pass project-default warning=5, critical=20 (or whatever the Stitch sheet specifies; verify before implementation)

**Acceptance:** the meter renders on every Dev Run detail page that has cost data; renders empty state for runs with `cost_usd === null` (in-progress runs that haven't accumulated cost yet).

### Phase 2.B — LLM Calls list cost column

**Files:**
- `dashboard/src/pages/LlmCalls.tsx` — add cost column
- `crates/mika-agent/src/server/dashboard_llm_calls.rs` (or wherever the LLM calls list endpoint lives — verify at implementation) — confirm `cost_usd` is in the list response. **If absent**: add it as a `cost_usd: Option<f64>` field, derived from the call's `input_tokens × provider_input_price + output_tokens × provider_output_price` calculation that already exists somewhere in the codebase (grep at implementation).

**Change:**
- Add a `cost` column to the calls table, rendered with `<CostMeter variant="chip" value={call.cost_usd} />`
- Position after the existing `tokens` column (verify column order against Stitch screen)
- Sortable: extend the existing `sort` query parameter handling to include `cost` if not already
- Filterable: deferred to a follow-up unless trivial

**Acceptance:** every row in the LLM Calls list shows a cost chip; sort by cost works; the chip variant matches the Stitch design.

**Pre-flight (architect Phase 0 Pin pattern — gate Phase 2.B):**

Before writing any LlmCalls.tsx changes, run:

```bash
grep -n "cost_usd\|cost\|tokens" crates/mika-agent/src/server/dashboard_llm_calls.rs
grep -n "input_tokens.*price\|provider.*price\|cost.*calc" crates/mika-agent/src/llm/
```

These greps answer (a) whether the API already exposes per-call cost (no backend work) or doesn't (Phase 2.B grows by ~30 lines of Rust), and (b) where the cost calculation already lives if a backend addition is needed (DRY rather than re-deriving).

### Phase 2.C — Agent detail today's-cost chip

**Files:**
- `dashboard/src/pages/AgentDetail.tsx` — add cost chip
- Possibly a new endpoint or extension of the existing agent detail endpoint to include `cost_today_usd` and `cost_7d_rolling_usd`

**Change:**
- Add a `<CostMeter variant="chip">` to the agent header showing today's cost
- Optionally add a 7-day rolling chip alongside (defer if backend doesn't already expose 7d rolling)
- Position per Stitch screen for Agent detail (the ticket doesn't pin a specific screen for this surface; verify at implementation, fall back to operator-friendly placement at top-right of the agent header)

**Pre-flight:**

```bash
grep -n "cost\|today" crates/mika-agent/src/server/dashboard_agents.rs
grep -n "Agent\|cost" dashboard/src/api/agents.ts
```

If the agent detail API doesn't expose `cost_today_usd`, this phase grows by a small backend addition (a SQL aggregate over `llm_calls` filtered by `agent_id` + 24h window). Pin the existing aggregate query patterns (e.g., the dev runs page already does similar aggregations) and reuse the SQL idiom.

## Phase 3 — Landing widget (Layer 2)

**File:** `dashboard/src/pages/Dashboard.tsx` (or the landing-page component mika#666 establishes — verify at implementation; the path may differ depending on mika#666's final shape)

**Change:**
- Add a `Cost` widget section to the home landing page
- Reuse `<CostTrendChart>` from `dashboard/src/components/` directly (no migration to packages/ui)
- Show: total cost today, total cost 7d, the trend chart, leaderboard of top-3 most expensive recent runs (if backend exposes; otherwise defer)
- Layout per Stitch screen `2443ccdf05d44295ade2f7c17574d7c3`

**Pre-flight (gate Phase 3):**

mika#666 is being implemented in parallel. Before starting Phase 3:

1. Check mika#666 PR status — if merged, read the landing page's widget-slot contract and use it. If still open on a branch, read the in-progress branch's widget-slot interface and use it.
2. If mika#666 establishes a `<DashboardWidget>` wrapper or grid system, Phase 3 wraps the cost section in it. If mika#666 leaves widget composition unstructured (just a sequence of sections), Phase 3 adds a section at the position the Stitch screen specifies.
3. If mika#666 hasn't merged or even pushed by Phase 3 implementation time, **Phase 3 halts** and surfaces a coordination question to Vincent. The widget must compose with whatever mika#666 establishes; building it standalone risks a second integration pass.

This is a real pre-flight gate, not a soft preference. Phase 1 + Phase 2 are independent of mika#666 and ship even if Phase 3 stalls.

## Phase 4 — Tests + docs

**Tests:**
- Phase 1's CostMeter unit tests already specified.
- Phase 2.A: snapshot test for Dev Run detail with cost rendered + empty state.
- Phase 2.B: snapshot test for LLM Calls list with cost column + sort-by-cost integration test.
- Phase 2.C: snapshot test for Agent detail with cost chip.
- Phase 3: snapshot test for landing widget; coordinates with mika#666's existing landing-page test fixture.
- All tests follow the project's existing patterns (`@testing-library/react` per `packages/ui/CLAUDE.md`).

**Docs:**
- `packages/ui/CLAUDE.md` — add `CostMeter` to the canonical primitive enumeration, sibling to TokenBudgetBar, with a one-line "use this for unbounded threshold-based cost displays; use TokenBudgetBar for bounded token usage."
- `mika/docs/design/north-star.md` — add a one-line cost visibility callout if the design doc tracks user-facing affordances at this granularity (verify at implementation; do not bloat north-star.md if it's strategic-only).
- `dashboard/CLAUDE.md` — note the new cost surfaces (Dev Run detail, LLM Calls cost column, Agent detail cost chip, landing cost widget) under a "Cost visibility" subsection.
- No CHANGELOG entries needed (per project convention; release-plz handles it).

## Phase 5 — Out-of-scope follow-ups (filed at PR-merge time)

After this PR merges, file two follow-up tickets:

1. **Layer 3 — engine-side per-run cost ceiling enforcement.** Target milestone: mika#8 (Agent reliability). Surface area: `crates/mika-agent/src/agent/run_loop.rs` cost-check at top of each iteration; configurable per-agent ceiling in identity.toml; pause-and-confirm or hard-abort policy decision; tests at run_loop integration level. Triggered when operator feedback (or audit) reports a cost-overrun in autonomous runs after Layer 1+2 visibility ships.

2. **Anomaly detection — "this run is N× the median cost for similar tickets."** Target milestone: mika#13 (current). Surface area: a SQL aggregate that groups by ticket label and computes median cost, plus a `<CostMeter>` extension that shows an anomaly badge. Defer until Layer 1+2 are in production and operator workflow surfaces the demand.

These follow-ups are filed at PR-close time, not now, because (a) the visibility surface doesn't exist yet so the demand is hypothetical, and (b) filing them now adds backlog noise without backing data.

## Acceptance criteria (from the ticket)

- [x] Cost is visible wherever a run/call/session is shown (not hidden behind a collapsible metadata blob). **Phase 2.A + 2.B + 2.C.**
- [x] A cost dashboard / widget shows aggregate burn and attribution. **Phase 3.** (Widget on home landing, not dedicated page — per Stitch design.)
- [x] Per-run cost meters exist on in-flight surfaces. **Phase 2.A.** (Dev Run detail surfaces in-flight runs with the meter rendering live cost as the run progresses, via existing `useDevRun` polling.)
- [x] Decision flagged: should the engine enforce per-run cost ceilings (separate ticket if yes). **Phase 5 follow-up filing.**

## Risks and known unknowns

- **Risk: mika#666 lands during Phase 3 implementation, changing the widget-slot contract.** Mitigation: Phase 3's pre-flight gate halts and coordinates if mika#666 hasn't shipped a stable contract. Phase 1 + 2 ship independently.
- **Risk: LlmCalls list endpoint doesn't expose `cost_usd`, requiring a backend addition that grows Phase 2.B's surface.** Mitigation: Phase 2.B's pre-flight gate verifies before implementing. If the addition is needed, the SQL idiom is already established in dev_runs.rs (the pin found `cost_usd` plumbed there).
- **Risk: Stitch screen for Agent detail isn't pinned in the ticket body.** Mitigation: Phase 2.C falls back to "operator-friendly placement at top-right of agent header" with a note to Vincent. Not a blocker.
- **Unknown: default warning/critical threshold values.** The ticket's body doesn't pin them; the Stitch component sheet may. Implementer reads the sheet at Phase 1 and uses those values; if absent, defaults to \$5 / \$20 with a comment that these are best-guess and operator should refine.
- **Unknown: 7d rolling cost on Agent detail — is it in the existing API or needs adding?** Phase 2.C pre-flight resolves. If adding, ~10 lines of Rust + SQL.
- **Risk: visual divergence from CostTrendChart's existing aesthetic.** CostTrendChart shipped with its own visual language (per PR #976). The new `<CostMeter>` may look different. Mitigation: Phase 1 reads both the CostTrendChart's existing styling and the new CostMeter's Stitch sheet, and surfaces any inconsistency to Vincent before shipping. Don't relitigate — surface.

## Compound learning to write at PR-close

A short compound at `mika/docs/solutions/best-practices/` covering: **"When to add a new packages/ui primitive vs reuse an existing one."** Pattern: domain semantics drive the primitive boundary, not visual similarity. TokenBudgetBar (bounded, ARIA `meter`) and CostMeter (unbounded, ARIA `status`) look superficially similar but have different ARIA roles, different threshold semantics, and different evolution trajectories. Cite the policy from `packages/ui/CLAUDE.md` ("hand-rolled implementations are review fails") and counterbalance with this principle ("but force-fitting an existing primitive on a domain mismatch is also a review fail").
