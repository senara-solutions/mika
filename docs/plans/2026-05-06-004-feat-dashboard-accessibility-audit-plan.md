---
ticket: mika#668
type: feat
title: Dashboard accessibility audit and CI gate
date: 2026-05-06
seq: 004
---

# Plan: dashboard accessibility audit and CI gate (mika#668)

## Why

The dashboard has not been explicitly audited for accessibility. Manual ARIA usage in `dashboard/src/pages/` is present in only 5 files (verified via `grep -l "aria-\|role=" dashboard/src/pages/ | wc -l = 5`), no a11y dependencies exist in either `packages/ui/package.json` or `dashboard/package.json`, and no CI a11y gate runs against new PRs. The dashboard ships dark-theme grey-on-darker-grey text in many surfaces; contrast against WCAG AA has never been measured. Keyboard navigation, screen reader support, focus management, and reduced-motion respect are all untested.

Three forces make this load-bearing now, not later:

1. **Shared UI package consumed downstream.** `@senara-solutions/ui` is consumed by mika-cloud. If mika-cloud serves any external users (B2B customers, on-prem operators), accessibility becomes a legal and commercial concern, not a nice-to-have.
2. **Operator ergonomics.** The dashboard is a daily-use surface for Vincent. Keyboard-only operability shortens the feedback loop materially.
3. **Cross-cutting amortization.** Every page-redesign ticket in milestone#13 (#654, #655, #657, #662, #664, #667, #672, #676) touches a11y concerns. Building a11y in during extraction is meaningfully cheaper than retrofitting after.

The fix bar is to ship the **audit + CI gate + canonical-primitive fixes** in this PR, with per-page remediation findings filed as separately-scoped follow-up issues. The CI gate is the durable artifact — it prevents regressions in everything that hasn't been audited yet (e.g., the surfaces #671 and #672 introduce after this PR merges).

## Phase 0 — Pin (verified state, source-anchored)

All paths verified against the worktree at `feat/668/dashboard-accessibility-audit` HEAD `48e52c83` (main).

### Existing a11y tooling — none

- **`packages/ui/package.json`** — devDependencies include `@testing-library/jest-dom` and `@testing-library/react` + `vitest` + `jsdom`. **No `axe-core`, `jest-axe`, or `@axe-core/playwright`.** Test runner is vitest. New a11y test infrastructure must integrate with vitest.
- **`dashboard/package.json`** — scripts: `dev`, `build`. No `test` script visible at the top of the scripts block (read more at implementation if needed). No a11y deps.
- **No `.eslintrc*` or `eslint.config*` files** in either `dashboard/` or `packages/ui/` root. No `eslint-plugin-jsx-a11y` configured. ESLint tooling decision is open: add minimal config alongside this PR or defer.
- **CI workflows** at `.github/workflows/` (verify at implementation): no existing a11y job. New CI gate is greenfield.

### Existing primitives (a11y-relevant)

- **`packages/ui/src/components/StatusBadge.tsx`** — used pervasively. A11y baseline unknown; needs audit.
- **`packages/ui/src/components/ListRow.tsx`** — three-variant row primitive (static/navigable/expandable). Per `packages/ui/CLAUDE.md`: navigable variant has row-level `onClick`. Keyboard handler needs verification.
- **`packages/ui/src/components/SelectFilter.tsx`** + **`AgentFilter.tsx`** — categorical filter dropdown. Native `<select>` or custom widget? Keyboard semantics differ materially.
- **`packages/ui/src/components/TimeRangeFilter.tsx`** — presets + custom date-picker. Custom date pickers are a known a11y minefield.
- **`packages/ui/src/components/LoadingState.tsx`** — has `ariaLabel` prop per the canonical-primitive table; needs verification it's actually announced.
- **`packages/ui/src/components/ErrorState.tsx`** — has retry + details affordances per the table; verify keyboard reachability.
- **`packages/ui/src/components/EmptyState.tsx`** — extended with optional action affordance; verify focus management.
- **`packages/ui/src/components/CopyButton.tsx`** — icon button. Likely needs aria-label and a copy-confirmation announcement (live region).
- **`packages/ui/src/components/TokenBudgetBar.tsx`** — verified at line 48: `role="meter"` with aria-valuenow/min/max/label. Source-anchored a11y; baseline good.
- **`packages/ui/src/components/Pagination.tsx`** — keyboard navigation between pages, current-page semantics.
- **`packages/ui/src/components/MarkdownContent.tsx`** — uses `react-markdown` + `remark-gfm`. Renderer's a11y is library-determined; verify whether tables/links inherit semantic markup.
- **`LiveRefreshToggle`** (newly added — canonical primitive enumeration in `mika/CLAUDE.md` line 33). Toggle switch + LIVE badge. ARIA pattern for toggle state needed.

### Dashboard-side surfaces

- **5 page files use `aria-*` or `role=`** (per `grep -l "aria-\|role=" dashboard/src/pages/ | wc -l`). Pin which 5 at audit time. The other ~20+ pages have no explicit a11y markup — relying on default semantic HTML (which may or may not be sufficient).
- **`dashboard/src/components/CostTrendChart.tsx`** — chart visualization (PR #976). Charts need text alternatives for screen readers (data table, summary, or aria-describedby). Verify at audit time.
- **`dashboard/src/pages/Dashboard.tsx`** (mika#666 landing, just shipped via PR #989) — new surface; widget composition's keyboard order matters.

### Sibling milestone#13 dependencies

The cascade has already shipped or is shipping:
- mika#654 (ListRow extraction) — shipped
- mika#655 (SelectFilter / AgentFilter extraction) — shipped
- mika#656 (TokenBudgetBar) — shipped
- mika#657 (StatusBadge audit) — shipped
- mika#658 (lifecycle states LoadingState/ErrorState/EmptyState) — shipped
- mika#659 (TimeRangeFilter) — shipped
- mika#660 (CostTrendChart) — shipped via PR #976
- mika#661 (task tree visualization) — shipped via PR #982
- mika#662 (live-refresh consistency, LiveRefreshToggle) — shipped via PR #990 (just merged 09:00:48Z this morning)
- mika#666 (landing page) — shipped via PR #989 (just merged 08:26:02Z)

**Still in flight or queued:**
- mika#664 (URL state) — in_progress
- mika#667 (cost signals) — groomed, queued behind #664
- mika#671 (LLM bodies to Langfuse, backend) — queued
- mika#672 (LLM bodies display, frontend) — queued
- mika#676 (session detail tabs URL sync) — queued

**Implication:** the audit (Phase 1 of this plan) catches the *current* state of the dashboard, which includes ~80% of milestone#13's UI work. The CI gate (Phase 4) prevents regressions in the remaining 20% (#664, #667, #671, #672, #676) and in everything beyond milestone#13. The audit findings inform what those tickets should NOT do.

## Scope

**In scope:**

- **Phase 1** — Audit (manual keyboard walk + axe-core automated + WCAG AA contrast pass) producing a finding catalog.
- **Phase 2** — Triage findings: each finding gets a disposition (fix-here / file-follow-up / accept-with-rationale).
- **Phase 3** — Apply fix-here remediations, scope-bounded to canonical primitives in `packages/ui/`.
- **Phase 4** — CI gate: vitest + `jest-axe` (or `axe-core` direct) for primitive-level component tests; Playwright + `@axe-core/playwright` for page-level smoke tests if Playwright already exists in the codebase, otherwise vitest+jsdom only with a follow-up ticket for E2E.
- **Phase 5** — Documentation: `packages/ui/CLAUDE.md` a11y standards section; `dashboard/CLAUDE.md` a11y conventions if dashboard-specific patterns emerge.
- **Phase 6** — Out-of-scope follow-ups filed at PR-merge time.

**Out of scope (explicitly):**

- **Per-page deep-dive remediation.** The audit catalogs page-level findings; Phase 2 triages them; Phase 3 only fixes those that live in primitives. Page-level fixes that aren't primitive-driven are filed as follow-up issues. Reasoning: a single PR that touches every dashboard page is unmergeably wide and merge-conflicts with every other in-flight cascade ticket. Per-page fixes scoped to follow-ups can be picked up in any order without coordinating.
- **Color palette redesign.** If contrast findings reveal the design system itself fails WCAG AA in some token combinations, this PR documents the finding but does not redesign tokens. The design system is owned by Vincent (per `mika/CLAUDE.md`: "The rulebook is owned by Vincent and updated via direct commits, not PRs"). A token-level fix is filed as a follow-up to Vincent.
- **Screen reader vendor-specific bugs.** If a particular screen reader (NVDA/JAWS/VoiceOver) has known bugs that affect rendering, that's a vendor concern not an a11y bug in our code. Document and move on.
- **Internationalization / RTL support.** Distinct concern from a11y. Out of scope unless an a11y finding incidentally touches LTR-assumed code.

**Position on Phase 3 scope-bounding (defended explicitly):**

The ticket body proposes "Catalog issues found. File each as a sub-issue or bundle by category depending on volume." This plan picks a side: **bundle the primitive-level fixes here; file every page-level fix as a separate issue.** Reasoning:

1. **Merge-conflict risk.** Per-page a11y fixes to `LlmCalls.tsx`, `DevRunDetail.tsx`, etc. directly conflict with #664 (URL state, touches list pages) and #672 (LLM bodies display, touches LLM Calls page). Bundling page-level fixes in this PR creates blocking conflicts with two queued tickets.
2. **Audit-then-fix-then-audit cycle.** Page-level fixes are best applied as part of the page's redesign cycle, not in a centralized batch. The audit findings document what each page needs; the next-touch of each page applies the fix.
3. **CI gate is the durable artifact.** Even if zero per-page fixes ship in this PR, the CI gate catches regressions in everything subsequent. The audit's value is durable through the gate, not through batched fixes.
4. **Primitive-level fixes amortize.** `<CopyButton>` aria-label fix amortizes across every page that uses it. `<ListRow>` keyboard activation fix amortizes across every list page. These are high-leverage; per-page fixes are not.

This is a deliberate scoping decision, not a hedge. The plan commits to it.

## Phase 1 — Audit

### 1.A — Automated audit with axe-core (in vitest+jsdom)

**Approach:** `jest-axe` (or `@axe-core/react` + a vitest matcher) against rendered components in `packages/ui/src/components/*.test.tsx`. Each canonical primitive gets an axe assertion alongside its existing tests.

**Pre-flight:**
- Verify `jest-axe` is compatible with vitest (it should be; both follow Jest matcher conventions). If not, fall back to `axe-core` direct usage with a custom vitest matcher.
- Add `jest-axe` (or `axe-core`) to `packages/ui/package.json` devDependencies.
- Add `@types/jest-axe` if using TypeScript matchers.

**Deliverable per primitive:**
```ts
import { axe } from 'jest-axe'
expect.extend(toHaveNoViolations)

it('has no axe violations', async () => {
  const { container } = render(<Primitive {...defaultProps} />)
  const results = await axe(container)
  expect(results).toHaveNoViolations()
})
```

Add to: StatusBadge, TaskStatusBadge, Pagination, LoadingState, EmptyState, ErrorState, CopyButton, MarkdownContent, ListRow, SelectFilter, AgentFilter, TimeRangeFilter, TokenBudgetBar, LiveRefreshToggle. (14 components.)

**Output:** a list of axe violations per primitive. Some will pass (e.g., TokenBudgetBar with its source-pinned ARIA setup); others may have findings.

### 1.B — Manual keyboard walkthrough

**Approach:** the implementer (whoever picks this up) opens the dashboard locally, runs through every page using only keyboard. Document each surface as pass / fail with specifics.

**Surfaces to walk:**
- Landing (Dashboard.tsx — mika#666)
- Dev Runs list, Dev Run detail
- Team Runs list, Team Run detail
- Tasks list, Task detail
- LLM Calls list, LLM Call detail
- Sessions list, Session detail (with all tabs)
- Agents list, Agent detail
- Events / Webhook DLQ pages
- Any modals or drawers

**Checklist per surface:**
- Tab order is logical (top-to-bottom, left-to-right)
- All interactive elements reachable via Tab
- Visible focus indicator on every focused element
- Enter/Space activates buttons and rows
- Escape dismisses modals/drawers
- Filter controls have keyboard equivalents for all mouse interactions

**Output:** markdown table per surface with pass/fail + notes. Saved to `mika/docs/audits/2026-05-06-dashboard-a11y-audit.md` (new file in a new directory; the directory is created with this PR).

### 1.C — Color contrast (WCAG AA: 4.5:1 body, 3:1 large text)

**Approach:** programmatic check of every color-token pair used in the dashboard. The contrast ratio between foreground and background tokens in `packages/ui/src/theme.css` is computable; an automated check is more reliable than manual sampling.

**Tooling:** existing CSS color tokens from `theme.css` + a small Node script (committed alongside the audit doc as `scripts/check-contrast.mjs`) using `wcag-contrast` npm package or hand-rolled luminance calculation.

**Output:** matrix of token pairs with ratios; flag any below threshold.

### 1.D — Reduced motion

**Approach:** grep for `transition`, `animate`, `keyframes`, `auto-refresh` in dashboard + ui CSS and JS. For each, verify a `prefers-reduced-motion` guard exists.

**Quick smoke:** add `@media (prefers-reduced-motion: reduce) { * { animation: none !important; transition: none !important; } }` to the dashboard's debug stylesheet and visually walk through; document any motion that violates.

**Output:** grep results + smoke-test notes appended to the audit doc.

### 1.E — Text zoom / reflow (WCAG AA at 200%)

**Approach:** browser zoom at 200% on every list and detail page. Document any horizontal scroll, content cut-off, or layout breakage.

**Output:** screenshots + notes appended to the audit doc.

### Audit document deliverable

Single file: `mika/docs/audits/2026-05-06-dashboard-a11y-audit.md`. Sections:
- Methodology (what was tested, what tools, what surfaces)
- Automated findings (axe-core results)
- Keyboard walkthrough (per-surface pass/fail table)
- Contrast matrix
- Reduced motion findings
- Text zoom findings
- Severity classification (Critical / Serious / Moderate / Minor per axe convention)
- Disposition column for each finding (filled in Phase 2)

## Phase 2 — Triage findings

For each finding from Phase 1, assign one of three dispositions:

- **fix-here** — primitive-level issue with high leverage (touches `packages/ui/`). Fixed in Phase 3.
- **file-follow-up** — page-specific issue, design-system token issue, or cross-cutting work that exceeds this PR's scope. Filed as a separate issue in Phase 6. Each follow-up issue has: severity, surface, repro, proposed fix.
- **accept-with-rationale** — the finding is intentional or out-of-scope (e.g., a known browser quirk, a design-system decision Vincent owns). Documented in the audit doc with a one-line rationale.

The disposition column in the audit doc is the durable record. Triage decisions are not buried in conversation.

**Halt threshold:** if Phase 1 surfaces more than **15 fix-here findings**, the plan halts and surfaces to operator. 15 is the soft threshold for "this PR is no longer a tightly-scoped audit; it's a remediation sprint." Above the threshold, the plan should split: ship the audit + CI gate as the current PR; file the fix-here findings as their own ticket. Below the threshold, proceed.

## Phase 3 — Apply fix-here remediations

**Files touched:** primitives in `packages/ui/src/components/` per Phase 2 dispositions. Tests in `packages/ui/src/components/*.test.tsx`.

**Common likely fixes (predicted, not pinned — the audit determines which actually apply):**

- `<CopyButton>` — add aria-label (probably already has one; verify). Add `aria-live="polite"` region for the "Copied!" confirmation, or use `aria-pressed` state.
- `<ListRow variant="navigable">` — verify keyboard handler (`onKeyDown` with Enter/Space → onClick). Verify `role="link"` or `role="button"` on the row.
- `<SelectFilter>` — if custom widget, verify ARIA combobox pattern. If native `<select>`, verify label association.
- `<TimeRangeFilter>` — date picker is the highest-risk component; verify date input semantics, label associations, keyboard navigation between presets and custom inputs.
- `<MarkdownContent>` — verify external link `target="_blank"` carries `rel="noopener noreferrer"` (security + a11y).
- `<LiveRefreshToggle>` — verify toggle has `role="switch"` with `aria-checked={isLive}`, or is a native checkbox styled as a toggle.

**Each fix has:**
- A test that asserts the specific a11y property (e.g., `expect(button).toHaveAttribute('aria-label')`)
- An axe assertion in the component's test file
- A line in the audit doc's disposition column ("fix-here, applied @ <commit>")

## Phase 4 — CI gate

**Tooling decision:** `jest-axe` for vitest unit tests (Phase 1.A's existing infrastructure); skip Playwright E2E this PR.

**CI workflow change:** add a step to `.github/workflows/ci.yml` (verify exact path at implementation) that runs the packages/ui test suite with `--coverage` (already there?) and ensures axe assertions are run alongside. Most existing CI configs already run `npm test` or `vitest run` in the ui package's check job — verify whether the existing job covers the axe assertions or whether a dedicated `a11y-check` job is warranted.

**Pre-flight:**

```bash
ls .github/workflows/
grep -l "packages/ui\|vitest\|test" .github/workflows/*.yml
```

If the existing CI job already runs `npm test --prefix packages/ui`, the axe assertions ride along automatically. No new job needed. If the existing CI doesn't run packages/ui tests, add a job.

**E2E follow-up:** file an issue for "Add Playwright + @axe-core/playwright for page-level a11y E2E in CI." Out of scope for this PR; the unit-level axe via jest-axe is sufficient as a baseline gate.

**Hard halt threshold (sibling of #667 Phase 4):** if adding the CI job + tooling exceeds **40 lines** of YAML + config across all workflow files, halt and surface to operator. 40 lines is the soft signal that "this is more than a unit-test addition; it's CI infrastructure work" and may benefit from being its own ticket. Sub-40-line additions proceed without halt.

## Phase 5 — Documentation

**Files updated:**

- **`packages/ui/CLAUDE.md`** — new "## Accessibility standards" subsection documenting:
  - Every primitive must have an axe assertion in its test file (CI-enforced).
  - Every interactive primitive must have keyboard handlers tested.
  - Every icon-only button must have aria-label.
  - Every async state change must use a live region.
  - Color contrast must reference theme tokens, not hardcoded colors.
  - When introducing a new primitive, the a11y review-fail criteria are: missing axe test, no keyboard handler, hardcoded color, missing aria-label on icon-only button.

- **`dashboard/CLAUDE.md`** — note the audit doc path and the CI gate; one-line note that page-specific findings are filed as follow-up issues.

- **`mika/docs/audits/README.md`** (new) — establishes the audits/ directory convention. One-paragraph note: "audits/ contains time-stamped audit reports for cross-cutting concerns (a11y, security, performance). Each audit has a methodology section and a finding catalog with dispositions. Findings flagged as `file-follow-up` link to GitHub issues."

## Phase 6 — Out-of-scope follow-ups (filed at PR-merge time)

For each `file-follow-up` finding from Phase 2, file a GitHub issue at PR-merge time with:
- Title: `a11y(<surface>): <one-line description>`
- Severity: Critical / Serious / Moderate / Minor (axe convention)
- Repro: page URL or component path + reproduction steps
- Proposed fix: from audit doc disposition or "TBD"
- Reference: this PR's audit doc + commit SHA

Additionally, file:

1. **"Add Playwright + @axe-core/playwright for page-level a11y E2E"** — extends the unit-level axe gate to full-page E2E. Target milestone: open (depends on whether E2E infrastructure is being prioritized).

2. **"Design system a11y review — color token contrast under WCAG AA"** if Phase 1.C surfaces token-level contrast failures. Target: Vincent (design system owner per `mika/CLAUDE.md`).

3. **"jsx-a11y ESLint rule set"** — adds `eslint-plugin-jsx-a11y` to the dashboard + ui ESLint configs as a pre-CI gate. Out of scope this PR (no ESLint config exists yet; adding one is its own ticket).

## Acceptance criteria (from the ticket)

- [x] Audit report exists, filed as part of this ticket. **Phase 1 produces `mika/docs/audits/2026-05-06-dashboard-a11y-audit.md`.**
- [x] Every finding is either fixed or filed as a follow-up issue. **Phase 2 triages each; Phase 3 fixes primitive-level; Phase 6 files page-level as follow-ups.**
- [x] CI gate runs a11y checks on every dashboard PR. **Phase 4 adds jest-axe assertions to packages/ui/ tests, run on every PR via existing CI job.**
- [x] `packages/ui/CLAUDE.md` documents the a11y standards for new components. **Phase 5.**

## Risks and known unknowns

- **Risk: audit surfaces more than 15 fix-here findings.** Phase 2's halt threshold catches this. If hit, the plan splits into two tickets. Likelihood: medium — primitives have not been a11y-reviewed before.
- **Risk: jest-axe is incompatible with vitest.** Mitigation: Phase 1.A pre-flight verifies; falls back to `axe-core` direct with a custom vitest matcher (~10 lines). Likelihood: low — `jest-axe` is widely used with vitest.
- **Risk: design-system color tokens fail WCAG AA.** This is a finding that gets filed to Vincent, not fixed in this PR. The audit documents it; Phase 6 files the follow-up. Likelihood: medium — dark themes with grey-on-darker-grey often fail.
- **Risk: existing primitives have keyboard-handler bugs that fixing breaks consumer pages.** Mitigation: each Phase 3 fix has a snapshot test for the primitive AND a smoke test of at least one consuming page (read existing tests for that page; if a snapshot exists, the existing snapshot catches regressions). Likelihood: low — primitives are well-tested per the canonical-primitive table audits.
- **Risk: merge conflicts with #664 (URL state) and #672 (LLM bodies display) which touch list pages.** Mitigation: Phase 3's scope-bounded-to-primitives explicitly avoids per-page edits. Phase 1's audit may surface findings IN those pages, but those become follow-ups, not in-PR fixes. Likelihood: low given the scoping discipline; high if scoping is violated.
- **Unknown: how existing dashboard pages handle modals/drawers.** Phase 1.B's keyboard walkthrough surfaces this. Common a11y bug: focus trap missing in modals. If found, the fix is primitive-level (a `<Modal>` or `<Drawer>` primitive with focus-trap, possibly via `react-focus-lock` or hand-rolled). May expand Phase 3's scope.
- **Unknown: whether Playwright E2E exists in the codebase.** Phase 4's pre-flight resolves. If yes, axe-core/playwright might be cheap enough to add here; if no, deferred to follow-up.

## Compound learning to write at PR-close

A short compound at `mika/docs/solutions/best-practices/a11y-audit-as-code-2026-05-06.md`. Title: **"Accessibility audit as code — capturing audit findings durably."** Principle: a11y audits decay rapidly when stored in conversation, Slack, or PR descriptions. The durable storage is (a) `docs/audits/` for the finding catalog with dispositions, (b) per-component test assertions for the fix-here findings, (c) follow-up issues for the file-follow-up findings, and (d) CI gates for prevention of regressions. Cite this PR as the canonical example and the audit doc as the canonical structure for future audits (security, performance, etc.).
