# Dashboard ↔ Stitch Reconciliation Map

**Status:** Active — first ecosystem-wide reconciliation, scoped to the Observability Dashboard.
**Closes:** senara-solutions/mika#669.
**Drives:** milestone #13 (Dashboard improvements) — provides the design references and gap list to execute against.
**Companion:** [`north-star.md`](./north-star.md), [`luminescent-core.md`](./luminescent-core.md). Every decision below is judged against those.
**Stitch project:** `projects/6562713725762717689` ("Mika Observability Dashboard"), 14 screens as of 2026-04-25.

---

## Mapping

Each Stitch screen is one of **Relevant** (use as-is or with rulebook adjustments), **Stale** (supplanted by a newer iteration; retire), or **Orphaned** (no clear ticket consumer; revisit when the consumer surfaces). Stale screens stay in the Stitch project for now (UI deletion deferred — out-of-scope to do programmatically; the MCP doesn't expose `delete_screen`); they are documented here as not authoritative.

| # | Screen | Stitch ID | Status | Maps to milestone #13 ticket(s) | Notes / rulebook adjustments |
|---|---|---|---|---|---|
| 1 | **Conversation Sessions Viewer** | `00939e3328984b249c0f5318da199fa0` | Relevant | Sessions page (intersects #676 URL state, #662 live-refresh) | Split list+content layout. Apply tonal-shift row separation; remove any 1px dividers. |
| 2 | **Team Runs History List** | `64fd458f548e436b80500a069f37bc47` | **Relevant — canonical** | **#652** (Team Runs telemetry, iteration nav) | Paginated table with Team / Status / Date filters. Drop heavy column borders → tonal-shift rows. |
| 3 | **Tasks & Orchestration Overview** | `976190aa2b314effb5a51ab8c581180e` | **Relevant — canonical** | **#666** (landing/home "current state of the world") + Tasks page | Stacked sections (Work Items / Team Runs / Standalone Callbacks / Scheduled Tasks). Strong candidate for the new home page concept. |
| 4 | **Agent Core Memory Widget** | `7705e941bd5d4f18adbc43e0d19cac6f` | **Relevant — canonical** | **#656** (Core Memory actionable) | Per-section blocks (USER_SUMMARY / SELF_MODEL / CURRENT_PRIORITIES / KEY_PEOPLE / WORKFLOWS) with token-usage indicators. Section labels should be uppercase tracking-wide per rulebook §3. |
| 5 | **Tool Call Data Table View** | `7dc4112340fd432b9f1fbde1be481728` | Relevant | Tool Calls page (intersects #653 LLM Calls linked tool calls, #663 pagination) | Status / Tool Name / Input / Output / Actions columns. Keep Export CSV. Apply tonal-shift row separation. |
| 6 | **Agents Overview & Details** | `2e9012604d5b4718b5ab7e055ebb63df` | **Relevant — canonical** | **#656** + Agents page | Split layout: agent list (left) + agent detail (right) with status pill, Core Memory raw JSON, Recent Audit Events, soul.md viewer. The Core Memory section here should consume the same widget defined in screen #4. |
| 7 | **Navigation Sidebar Update** | `993a00f29d174072bbab8a1eb2d768aa` | Relevant | Foundational nav primitive | Nav items: Event Timeline / Agents / Sessions / Traces / Tasks (New) / Team Runs (New) / Settings. Use as the canonical sidebar shape; promote to `@senara-solutions/ui` if shared with Cloud Console. |
| 8 | **Team Run Debug Detail** | `abe0ec4a059d459f94220fad9404149a` | **Relevant — canonical** | **#652** (Team Runs detail) + **#661** (task tree visualization) | Iteration Timeline + per-iteration phases (Assign / Execute) + per-agent task cards + Workspace Entries Log. The Iteration Timeline IS the answer to #661 task-tree visualization. |
| 9 | **Unified Event Timeline Dashboard** | `c5b6feddb5444f3d83a7f9b94e140bcd` | **Relevant — canonical** | Existing Event Timeline page; informs #662 (live-refresh — currently the only page with it), #659 (time range filter), #665 (copy feedback) | Live Monitor toggle, multi-filter (Agent / Event Type / Time Range / More), table with copy buttons. Most aligned today. The live-refresh + time-range patterns here are the templates for other surfaces. |
| 10 | Session Tool Trace Detail (variant a) | `6e8b01df2a40457e876cf10c9cba40e8` | **Stale** | — | Inline pipe-separated `IN: ... | OUT: ...` is unreadable. Supplanted. |
| 11 | **Session Tool Trace Detail (variant b)** | `7510d56d45844118bd815995e672bb2d` | **Relevant — canonical (LIST)** | **#653** (LLM Calls linked tool calls list), **#652** (Team Runs trace widget § list rows) | Table layout with explicit Status / Tool & Parameter / Input / Output columns. Canonical for *scanning many tool calls in sequence*. Apply tonal-shift row separation per rulebook §5. |
| 12 | Session Tool Trace Detail (variant c) | `dfa1c6b386fb442d92cb69cbdb3b8b1c` | **Stale** | — | Sentence-case `Input:` / `Output:` labels not aligned with rulebook §3 (labels must be uppercase tracking-wide). Supplanted by variant d. |
| 13 | **Session Tool Trace Detail (variant d)** | `e7cd46efd53b4c0d91e689edff4fa877` | **Relevant — canonical (DETAIL)** | **#653** (LLM Calls single tool-call drawer), **#661** (task tree expansion) | Stacked layout with uppercase tracking-wide `<h4>` INPUT / OUTPUT labels and pretty-printed JSON. Canonical for *one tool call in depth*. Already aligned with rulebook typography §3. |
| 14 | "Team Collaboration Detail" (mistitled — actually session detail) | `f6255ed0c86c4229892fa3c7cbe86776` | **Stale** | — | Title misleading: content is yet another Session Detail iteration. Plain `Input` / `Output` labels — less aligned than variant d. |

### Trace-detail role split (resolves the four-iteration ambiguity)

Variants b and d are both canonical because they solve **different jobs**, not competing roles:

- **List rendering** (scanning many tool calls): variant b. Built for density and scanning. Used in #653's tool-calls table and #652's trace widget rows.
- **Detail rendering** (drilling into one call): variant d. Editorial typography, pretty JSON, prominent system-metadata labels. Used in #653's drawer/page for a single call and #661's task-tree expansion.

Variants a, c, and screen #14 are intermediate iterations and stale.

---

## Gaps — milestone #13 tickets with no existing Stitch view

These need design work before their tickets can ship.

| Ticket | Gap | Why it matters | Proposed action |
|---|---|---|---|
| **#651 Dev Runs** | No Dev Runs page in Stitch. The Dashboard currently has a Dev Runs page that's broken (issue & PR fail to load, pipeline state inaccurate per the ticket). | Dev Runs is the second of the two big page redesigns (alongside #652 Team Runs). Without a design, the bug fixes in the ticket can't ride a coherent layout. | Stitch session to design Dev Runs detail + list. Reference patterns: Team Run Debug Detail (#8 above) shape, plus a CI-status / PR-status row. |
| **#658 Empty / loading / error states** | No state catalog exists. | Every page in milestone #13 needs to render these states; if they're not designed once and shared, every page redesign reinvents them. Foundational primitive. | Stitch session to design the canonical state set. Promote to `@senara-solutions/ui` (`<EmptyState>` already exists; extend it; add `<LoadingState>` and `<ErrorState>`). |
| **#660 Charts / time-series** | No chart screen. | Cost/budget signals (#667), event timeline trends, latency distributions all need charts. The ticket says "decide stance and scope" — this needs a visual answer. | Stitch session to propose 2-3 chart styles aligned with The Luminescent Core (dark, soft, low-density). |
| **#667 Cost / budget signals** | No cost-signal screen. | Hard dependency for surfacing mika-arch's Unit 8 cost-monitoring data once it lands; also surfaces general LLM cost across pages. | Stitch session to design the cost-signal pattern (threshold pill, in-page warning band, dashboard-level summary card). Hard-blocked on the chart decision (#660). |
| **#653 LLM Calls page itself** (the primary list view) | Variant b/d cover *tool-call rendering within* the LLM Calls detail, but the LLM Calls primary list page (request/response cards/rows, latency, model) is not in Stitch. | The ticket calls out "no prompt/response content, bland metrics, no linked tool calls" — those are all on the primary page, not just the detail. | Stitch session to design the LLM Calls list page with prompt/response previews, tool-call links, and metrics. Includes #672 (LLM bodies display). |

---

## Proposed sequence for milestone #13

Per the north star ("intuitive — depth one click away"), pages depend on primitives, primitives depend on tokens. Sequence accordingly:

### Phase 1 — Foundation (already done)
- ✅ **The Luminescent Core** — covers #657 (visual rhythm, status pills, spacing tokens). The rulebook IS the tokens layer.

### Phase 2 — Primitives in `@senara-solutions/ui`
Extract or build, all in `packages/ui/`. These unlock every page redesign that follows.

| Primitive | Tickets | Already in `@senara-solutions/ui`? |
|---|---|---|
| `<StatusBadge>` / `<StatusPill>` | #657 | Yes — audit against rulebook §5 chip spec. |
| `<Pagination>` | #663 | Yes — migrate hand-rolled instances per ticket body. |
| `<EmptyState>` + `<LoadingState>` + `<ErrorState>` | #658 | Partial (`<EmptyState>` exists). Extend. |
| `<CopyButton>` with visual confirmation | #665 | Yes — add visual confirmation per ticket. |
| `<ListRow>` (tonal-shift row, no borders) | #654 | No. Build, then migrate hand-rolled. |
| `<AgentFilter>` / unified filter primitives | #655 | No. Build. |
| `<TimeRangeFilter>` | #659 | No. Build. |
| `<TraceIdWidget>` (linkable trace pill that opens trace) | #652, #653 | No. Build. |

### Phase 3 — Page redesigns (consume primitives)
Order by dependency on phase 2 + design-gap prerequisites:

1. **#666 Landing / home** — has a canonical Stitch screen (Tasks Overview, #3). Builds on phase-2 primitives. **Start here** — anchors the rest of the milestone.
2. **#652 Team Runs** — canonical screens for both list (#2) and detail (#8) exist. No design gap.
3. **#656 Agent detail Core Memory** — canonical screens (#4 widget + #6 page) exist. No design gap.
4. **#661 Task tree visualization** — Iteration Timeline pattern from #8 covers it. No design gap.
5. **#651 Dev Runs** — **design gap**. Stitch session first, then implement. Also has a real bug (issue & PR fail to load) to fix in the same PR.
6. **#653 LLM Calls detail** + **#672 LLM bodies** — **design gap on primary page**. Tool-call rendering within (variant b/d) is ready. Stitch session for the primary list, then implement.
7. **#660 Charts / time-series** + **#667 Cost / budget signals** — **design gaps**. Sequenced after mika-arch v1 Unit 8 ships the cost-monitoring log fields. Stitch session, then implement.

### Phase 4 — Cross-cutting
- **#636 (p1 stale-WAL bug)** — orthogonal to design; can ship anytime. Not blocked on this map.
- **#662 Live-refresh consistency** — Event Timeline pattern (#9) is the template; apply to Team Runs, Tasks, Sessions.
- **#664 URL state** — non-visual; ship per page as the page is redesigned.
- **#668 Accessibility audit** — runs after phase 3 lands; final pass.
- **#669 (this ticket)** — closes when this map is committed.

---

## Workflow agreement (for this and the next two reconciliations)

The workflow we agree on for the Dashboard becomes the template for the Cloud Console and Landing Page reconciliations.

### Roles

- **Vincent** initiates Stitch sessions when a design gap (per the table above) is the next blocker, or when an existing screen needs revision against new context. Owns the rulebook; final say on aesthetic decisions.
- **Claude** brings: the ticket(s) being designed for, the primitive inventory (what's in `@senara-solutions/ui`), the relevant rulebook constraints, and any prior Stitch screens to iterate from. Produces this map (and its updates).
- **Stitch** generates and iterates screens via the MCP (`generate_screen_from_text`, `edit_screens`, `generate_variants`). Outputs are HTML + screenshots, addressable by screen ID.

### What Stitch produces

- New screens (HTML + screenshot) representing the proposed design.
- Variants when a decision-point needs visual comparison (e.g., the four trace-detail iterations).
- Design-system extensions when a screen uses a pattern the rulebook doesn't yet name.

### How outputs land in tickets

- The Stitch screen's URL or ID is added to the relevant milestone #13 ticket body as a "Stitch reference: `<screen-id>` (see `docs/design/dashboard-stitch-map.md` § <ticket>)" line. The map is the canonical source; the ticket gets a pointer for discoverability.
- Implementation PRs reference the Stitch screen ID in their body and assert which rulebook sections they apply.

### When a design is "accepted"

- Vincent says it's accepted in the session (verbal/text — no separate sign-off ceremony).
- The map is updated to reflect the new canonical screen. Any prior canonical for the same role is moved to **Stale**.
- The implementation ticket can start.

### When a primitive gets elevated to `@senara-solutions/ui`

Default rule from the north star: **if more than one surface needs it, it goes in `@senara-solutions/ui`.** Concretely:

- A primitive used by the Dashboard AND the Cloud Console (or any combination of two surfaces) → `@senara-solutions/ui`.
- A primitive used by only one surface → stays as a local component in that surface.
- Re-evaluation happens during each subsequent reconciliation pass (Cloud Console next, Landing Page after) — primitives that were Dashboard-local may get promoted then.

### When this map updates

- Direct commits to main (per the design-discipline rule "we don't make PRs for these kinds of changes").
- Append-only entries in the Status column of the inventory table; existing rows update in place when canonical decisions shift.
- Append-only entries to the gap list as new tickets surface.

---

## Status

- **2026-04-25:** Map created. Three stale variants (10, 12, 14) flagged for UI deletion when convenient. Workflow agreement codified. Phase 1 (tokens) done; phase 2 (primitives) ready to attack after mika-arch v1 (#811) lands.
