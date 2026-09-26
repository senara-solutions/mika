---
title: "feat(ui): audit Pagination usage, document canonical-primitive enforcement"
type: feat
status: active
date: 2026-04-26
origin: senara-solutions/mika#663
---

# Plan — dashboard Pagination audit (mika#663)

**Issue:** [mika#663](https://github.com/senara-solutions/mika/issues/663) — `Dashboard > Pagination: audit @senara-solutions/ui <Pagination /> usage, migrate hand-rolled instances`
**Branch:** `feat/663/dashboard-pagination-audit-migrate`
**Type:** feat (Phase 2 primitive in milestone #13)
**Labels:** enhancement, dashboard

## Problem (per issue body)

`@senara-solutions/ui` exports a `Pagination` component. Unclear whether every paginated table in the dashboard uses it; possible drift between hand-rolled and shared implementations. Issue calls for an audit, migration of any hand-rolled instances, and canonical-primitive naming so future hand-rolled pagination becomes a review fail.

## Audit results (verified during planning)

**The migration is already done.** Every paginated page imports `Pagination` from `@senara-solutions/ui`; zero hand-rolled instances.

**Two-part verification (per architect Finding 1, first-pass):**

```bash
# Part 1 — count callsites
grep -c "<Pagination" mika/dashboard/src/pages/*.tsx
# → 9 files, 15 callsites total

# Part 2 — confirm import source is the package (not a local path)
grep -rn "import.*Pagination" mika/dashboard/src/pages/*.tsx
# → all 9 files import from '@senara-solutions/ui'; zero local-path imports
```

Part 2 verification cited verbatim:
- `AgentDetail.tsx:4` — `import { StatusBadge, Pagination, EmptyState, MarkdownContent, formatRelativeTime } from '@senara-solutions/ui'`
- `LlmCalls.tsx:4` — `import { Pagination, EmptyState, formatTimestamp } from '@senara-solutions/ui'`
- `TeamRuns.tsx:3` — `import { Pagination, EmptyState, TaskStatusBadge, formatRelativeTime } from '@senara-solutions/ui'`
- `DevRuns.tsx:3` — `import { Pagination, EmptyState, TaskStatusBadge, formatRelativeTime } from '@senara-solutions/ui'`
- `SessionDetail.tsx:7` — `import { CopyButton, Pagination, EmptyState, formatTimestamp, getAgentColor } from '@senara-solutions/ui'`
- `ToolCalls.tsx:5` — `import { CopyButton, Pagination, EmptyState, formatTimestamp } from '@senara-solutions/ui'`
- `Tasks.tsx:4` — `import { Pagination, EmptyState, TaskStatusBadge, formatRelativeTime } from '@senara-solutions/ui'`
- `Timeline.tsx:5` — `import { Pagination, EmptyState, formatTimestamp, eventTypeBadge } from '@senara-solutions/ui'`
- `Sessions.tsx:4` — `import { Pagination, EmptyState, formatRelativeTime } from '@senara-solutions/ui'`

Both parts of the verification are clean. The "100% adoption" claim is grounded.

| Page | `<Pagination>` callsite count | `perPage` | `page` source |
|---|---|---|---|
| `dashboard/src/pages/Tasks.tsx` | 4 | 20 | local `useState` (multiple paginated sub-tables, all consistent) |
| `dashboard/src/pages/Sessions.tsx` | 1 | `filters.per_page ?? 50` | `useSearchParamsFilter` |
| `dashboard/src/pages/LlmCalls.tsx` | 1 | `filters.per_page ?? 50` | `useSearchParamsFilter` |
| `dashboard/src/pages/ToolCalls.tsx` | 1 | `filters.per_page ?? 50` | `useSearchParamsFilter` |
| `dashboard/src/pages/Timeline.tsx` | 1 | `filters.per_page ?? 50` | `useSearchParamsFilter` |
| `dashboard/src/pages/TeamRuns.tsx` | 1 | `filters.per_page ?? 50` | `useSearchParamsFilter` |
| `dashboard/src/pages/DevRuns.tsx` | 1 | `filters.per_page ?? 50` | `useSearchParamsFilter` |
| `dashboard/src/pages/AgentDetail.tsx` | 1 | 50 (hard-coded) | local `useState` |
| `dashboard/src/pages/SessionDetail.tsx` | 4 | 50 (hard-coded) | local `useState` (messages/llm-calls/tool-calls/audit-events sub-tables) |

**Total: 15 callsites across 9 pages. 100% use the shared component.** Visual consistency follows from sharing the component — `Pagination.tsx:14-37` renders the same structure for every callsite (`{total} total · page {page} of {totalPages}` + chevron buttons with identical Tailwind classes).

**Two non-drift variances worth naming:**

1. **`perPage` size differs:** Tasks uses `20`, AgentDetail and SessionDetail hard-code `50`, the URL-state pages use `filters.per_page ?? 50`. Not styling drift — just per-page sizing decisions reflecting list density. Documented as expected variance, not a migration target.
2. **`page`/`onPageChange` source mixes URL state and local state:** The URL-state pages use `useSearchParamsFilter`; AgentDetail/SessionDetail/Tasks use `useState`. URL-state-vs-local-state is mika#664's scope ("URL state: filters, sort, pagination should be URL-reflected for shareable links"). Out of scope for #663 — audit reports the variance, mika#664 owns the fix.

## Approach

Three minimal changes, all documentation:

### Change 1 — Add `packages/ui/CLAUDE.md`

**File:** `mika/packages/ui/CLAUDE.md` (new)

Per `mika/CLAUDE.md` Directory Structure ("packages/ui/ — `@senara-solutions/ui` shared React component library"), the package lacks a per-directory CLAUDE.md. Most other crates/dirs have one. Adding it now is the right place for:
- Component inventory (mirrors the root CLAUDE.md line, but with usage guidance)
- Canonical-primitive enforcement: "use these for X — hand-rolled instances are a review fail"
- Build/test/publish notes for Vite library mode + GitHub Packages

**Initial content scope (small, focused):**

```markdown
# @senara-solutions/ui — Shared React component library

## Stack
- Vite library mode (`vite-plugin-dts` for `.d.ts` generation)
- React 19 + TypeScript 5.x
- Tailwind CSS v4 (consumers import their own; this package emits utility-class strings, not bundled CSS)
- lucide-react for icons
- Published to GitHub Packages: `@senara-solutions/ui`

## Components

| Component | Use for | Hand-rolled forbidden | Migration status |
|---|---|---|---|
| `<StatusBadge />` | All status pills (agent state, run state, task state) | Yes | Audit pending (mika#657) |
| `<Pagination />` | All paginated tables/lists across the dashboard | Yes | Audited clean (mika#663) |
| `<EmptyState />` | All empty-list states; extend with `<LoadingState />` and `<ErrorState />` (#658) | Yes | Audit pending (mika#658) |
| `<CopyButton />` | All copy-to-clipboard affordances | Yes | Audit pending (mika#665) |
| `<MarkdownContent />` | Rendering Markdown content from agent messages, plan documents, etc. | Yes | Audit pending |
| `<TaskStatusBadge />` | Task-specific status pills (specialization of `StatusBadge` for task state vocabulary) | Yes | Audit pending (mika#657) |

**"Hand-rolled forbidden" means:** PR review fails on any new dashboard or surface code that re-implements one of these primitives. Reviewer instruction: "Use `<Component />` from `@senara-solutions/ui` instead of hand-rolling."

**"Migration status" means:** "Hand-rolled forbidden = Yes" expresses intent — the rule applies to all listed components. "Migration status" tracks whether an audit has confirmed zero residual hand-rolled instances. `Audited clean (mika#NNN)` rows are fully migrated; `Audit pending (mika#NNN)` rows still need their migration ticket to confirm. New components added to this table default to `Audit pending` until their migration ticket merges.

**Escape hatch — when a hand-rolled variant is approvable:**

The default is restrictive; exceptions exist but are bounded. PR review can approve a hand-rolled variant only when the justification names a missing capability:

- ✅ **Valid justification:** "The shared component's current API cannot serve this use case — it lacks `<missing-capability>`." The correct path is then a separate PR to extend `@senara-solutions/ui`, then use the extended shared component. The hand-rolled variant is acceptable only as a temporary bridge if the extension PR is filed and linked.
- ❌ **Invalid justification:** "I need this faster than a shared-component PR would take." Timeline pressure normalizes debt. A hand-rolled instance shipped under schedule pressure becomes the next ticket's migration target.

The judgment call lives at review time, not at authoring time, so the reviewer can see the full PR context (does this actually need the missing capability? is the extension PR linked?) before approving.

## Build / publish

[Inherits from root CLAUDE.md commands — `npm run build`, `publish-ui.yml` workflow, etc.]
```

### Change 2 — Update `mika/CLAUDE.md` Directory Structure entry

**File:** `mika/CLAUDE.md`

The existing line says "Components: StatusBadge, Pagination, EmptyState, CopyButton, MarkdownContent, TaskStatusBadge." Tighten this to: "Components: StatusBadge, Pagination, EmptyState, CopyButton, MarkdownContent, TaskStatusBadge. **Hand-rolled implementations of these are review fails — see `packages/ui/CLAUDE.md`.**"

One sentence added. Backward-compatible.

### Change 3 — Audit-result note in mika#663 issue body

Already covered by the body callouts at the top of this plan. No additional work — the audit table above is the issue's AC1 deliverable.

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `mika/packages/ui/CLAUDE.md` (new) | +30 lines: component inventory + canonical-primitive enforcement + build/publish notes |
| 2 | `mika/CLAUDE.md` | +1 sentence: review-fail callout pointing to `packages/ui/CLAUDE.md` |

Net diff: ~31 lines, 2 files. Smallest plan of the night.

## Tests

Documentation-only changes — no test scaffolding. Verification is by review:

1. Reviewer reads `packages/ui/CLAUDE.md` and confirms it accurately reflects current component inventory.
2. Reviewer reads `mika/CLAUDE.md` updated line and confirms the cross-reference is correct.
3. Manual `git grep` confirms no hand-rolled pagination remains (already true — see Audit Results table above).

## Acceptance criteria

- [ ] `mika/packages/ui/CLAUDE.md` exists with component inventory + canonical-primitive enforcement table.
- [ ] `mika/CLAUDE.md` Directory Structure entry for `packages/ui/` includes the review-fail callout pointing to `packages/ui/CLAUDE.md`.
- [ ] Audit table from this plan attached to the issue (already inlined here; transferred verbatim to closing comment when PR ships).
- [ ] `git grep -rn "page [0-9] of [0-9]" mika/dashboard/src/` returns only callsites that use the shared component (already true; verification step locks the invariant).

## Out of scope

- **Migrating hand-rolled instances** — there are none. Issue body anticipated this might be needed; audit confirmed it isn't.
- **Visual consistency tweaks** — all callsites already use the shared component, which renders identically. Any visual concern is a `Pagination.tsx`-level change, not per-callsite.
- **`perPage` size unification** — variances reflect list-density decisions per page; not styling drift.
- **URL-state migration for `page` parameter** — that's mika#664's scope ("URL state: filters, sort, pagination should be URL-reflected for shareable links"). #663 audits the current state; #664 enforces URL-reflectivity.
- **`<Pagination />` API changes** (e.g., custom labels, page-size selector) — YAGNI; current API serves all 15 callsites.
- **Adding the same enforcement to `mika-cloud/` or `mika-platform/.claude/commands/`** — those don't consume `@senara-solutions/ui` directly. The dashboard is the primary consumer. If/when the Cloud Console (per `docs/design/dashboard-stitch-map.md` future-reconciliation note) starts consuming `@senara-solutions/ui`, the same enforcement applies via this CLAUDE.md.

## Risks

| Risk | Mitigation |
|---|---|
| Component inventory in `packages/ui/CLAUDE.md` drifts from actual exports | Reviewer cross-checks `packages/ui/src/index.ts` against the table on every PR that touches `packages/ui/`. (Future: a CI step verifying inventory consistency, but YAGNI for v1.) |
| "Hand-rolled forbidden" framing is too strict — sometimes a one-off variant is justified | The framing is "review fail by default." Reviewer can approve a hand-rolled variant with explicit justification in the PR description. The default is restrictive; exceptions are documented exceptions. |
| Adding `packages/ui/CLAUDE.md` triggers a doc-sync hook (per `crates/mika-agent/build.rs` doc-sync convention) | The doc-sync hook syncs `docs/`, not `packages/ui/`. Verified by reading `mika/CLAUDE.md` § Conventions ("Doc sync: docs/ is the single source of truth ... `crates/mika-agent/build.rs` copies docs into OUT_DIR"). `packages/ui/CLAUDE.md` is outside that scope. |

## Sequencing

1. **Change 1 first** (add `packages/ui/CLAUDE.md`). New file, additive.
2. **Change 2 second** (update root `mika/CLAUDE.md` to reference Change 1).
3. **Manual verification**: re-run the audit grep (`grep -rn "page [0-9] of [0-9]" mika/dashboard/src/`) to confirm no new hand-rolled instances appeared during in-flight work.
4. **Open PR** cross-referencing #663. PR description includes the audit table verbatim.

## Verification

```bash
# Confirm packages/ui/CLAUDE.md exists and has the expected sections
ls mika/packages/ui/CLAUDE.md
grep -c "Hand-rolled forbidden" mika/packages/ui/CLAUDE.md  # → 1

# Confirm root CLAUDE.md cross-references it
grep "packages/ui/CLAUDE.md" mika/CLAUDE.md  # → match

# Confirm no hand-rolled pagination — usage count
grep -rn "page [0-9] of [0-9]\|currentPage" mika/dashboard/src/  # → 0 (or only inside Pagination.tsx if that bubbles up)

# Confirm import source — every callsite imports from the package, not a local path
grep -rn "import.*Pagination" mika/dashboard/src/pages/*.tsx  # → all 9 lines reference '@senara-solutions/ui'
```

## Discovery items (verified during planning)

1. **Audit complete: 100% adoption.** 9 dashboard pages × 15 callsites all use `<Pagination>` from `@senara-solutions/ui`. Confirmed by `grep -c "<Pagination" dashboard/src/pages/*.tsx`.
2. **No `packages/ui/CLAUDE.md` exists.** Need to create it for canonical-primitive enforcement to live somewhere durable. Found via `find packages/ui/ -name CLAUDE.md`.
3. **Root `mika/CLAUDE.md` lists components but without enforcement framing.** One sentence addition makes the inventory load-bearing.
4. **`<Pagination />` component is well-formed.** Read at `packages/ui/src/components/Pagination.tsx`: 4-prop API (`page`, `perPage`, `total`, `onPageChange`), ChevronLeft/ChevronRight icons, consistent Tailwind classes, `totalPages <= 1` short-circuit.
5. **`perPage` variance is intentional, not drift.** Tasks.tsx uses 20 (denser list); detail-page sub-tables use 50; URL-state pages use `filters.per_page ?? 50`. Variance reflects per-page sizing decisions.
6. **`page` source variance is mika#664's scope.** URL-state vs local-state is the URL-reflectivity question, separately tracked.
