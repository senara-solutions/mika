---
ticket: mika#668
type: feat
title: Dashboard accessibility audit and CI gate
date: 2026-05-06
seq: 004
---

# Plan: dashboard accessibility audit and CI gate (mika#668)

## Verified state (post-architect-pass-1)

- **F1 (Phase 0 Pin file enumeration) addressed** — Phase 0 now pins the exact 12 primitive `.tsx` files in `packages/ui/src/components/`, the test runner config (vitest 3.2.1, no separate config file — uses defaults + jsdom env), the test invocation line (`npm run test --prefix packages/ui` → `vitest run`), and the current devDependencies block verbatim. Phase 1.A's scope discovery: **only 1 test file exists** (`TokenBudgetBar.test.tsx`), so 11 of 12 primitives have no tests at all — Phase 1.A creates the first test file for each.
- **F2 (Phase 3 scope boundary mechanical rule) addressed** — Phase 2 triage now states a mechanical rule: a finding is `fix-here` candidate **if and only if** its source path is under `packages/ui/src/components/`. Findings with source paths under `dashboard/src/` (any subdirectory) are mechanically `file-follow-up` regardless of severity. The judgment-dependent "primitive-level / page-level" framing is gone.
- **F3 (15-finding threshold calibration note) addressed** — Phase 2's halt threshold now explains the 15 number: ~13 axe assertions (12 primitives + LiveRefreshToggle if rebased) plus expected ARIA-label/keyboard-handler fixes per primitive averages 1-2 findings each → ~15-25 fix-here candidates is the natural ceiling for "audit-with-fixes." Above that, the PR is a remediation sprint and benefits from being its own ticket.
- **F4 (`docs/audits/` greenfield check) addressed** — verified via pre-flight `ls mika/docs/audits/` → directory does not exist. Phase 1's audit doc creates it as new. No existing convention to inherit; Phase 5's `audits/README.md` establishes the convention.
- **U3 (focus-trap explicit disposition) addressed** — Phase 2's triage rules now state: modal/drawer focus-trap findings are **always** `file-follow-up`, regardless of severity, because the fix requires designing a new `<Modal>`/`<Drawer>` primitive (focus-lock). New-primitive design is out of this PR's audit-and-fix scope.
- **Important post-pin discovery (not from architect):** my worktree branched from main BEFORE PR #990 (mika#662, merged 09:00:48Z) added `LiveRefreshToggle.tsx`. The 12 `.tsx` files I pinned are pre-PR-#990 state. At dispatch time, the worktree rebases onto main and picks up LiveRefreshToggle, making it 13 primitives. Phase 1.A explicitly handles this: enumerate primitives by `ls packages/ui/src/components/*.tsx | grep -v .test.` at implementation time, not by hard-coding the list from this plan.

## Why

The dashboard has not been explicitly audited for accessibility. Manual ARIA usage in `dashboard/src/pages/` is present in only 5 files (verified via `grep -l "aria-\|role=" dashboard/src/pages/ | wc -l = 5`), no a11y dependencies exist in either `packages/ui/package.json` or `dashboard/package.json`, and no CI a11y gate runs against new PRs. The dashboard ships dark-theme grey-on-darker-grey text in many surfaces; contrast against WCAG AA has never been measured. Keyboard navigation, screen reader support, focus management, and reduced-motion respect are all untested.

Three forces make this load-bearing now, not later:

1. **Shared UI package consumed downstream.** `@senara-solutions/ui` is consumed by mika-cloud. If mika-cloud serves any external users (B2B customers, on-prem operators), accessibility becomes a legal and commercial concern, not a nice-to-have.
2. **Operator ergonomics.** The dashboard is a daily-use surface for Vincent. Keyboard-only operability shortens the feedback loop materially.
3. **Cross-cutting amortization.** Every page-redesign ticket in milestone#13 (#654, #655, #657, #662, #664, #667, #672, #676) touches a11y concerns. Building a11y in during extraction is meaningfully cheaper than retrofitting after.

The fix bar is to ship the **audit + CI gate + canonical-primitive fixes** in this PR, with per-page remediation findings filed as separately-scoped follow-up issues. The CI gate is the durable artifact — it prevents regressions in everything that hasn't been audited yet (e.g., the surfaces #671 and #672 introduce after this PR merges).

## Phase 0 — Pin (verified state, source-anchored)

All paths verified against the worktree at `feat/668/dashboard-accessibility-audit` HEAD `48e52c83` (main).

### Existing a11y tooling — none (verified)

- **`packages/ui/package.json` devDependencies** (verbatim at HEAD `48e52c83`):
  ```json
  {
    "@testing-library/jest-dom": "^6.6.3",
    "@testing-library/react": "^16.3.0",
    "@types/react": "^19.2.7",
    "@types/react-dom": "^19.2.3",
    "@vitejs/plugin-react": "^5.1.1",
    "jsdom": "^26.1.0",
    "lucide-react": "^0.575.0",
    "react": "^19.2.0",
    "react-dom": "^19.2.0",
    "typescript": "~5.9.3",
    "vite": "^7.3.1",
    "vite-plugin-dts": "^4.5.4",
    "vitest": "^3.2.1"
  }
  ```
  **No `axe-core`, `jest-axe`, `@axe-core/playwright`, `@axe-core/react`, or `eslint-plugin-jsx-a11y`.**
- **`packages/ui/package.json` scripts**: `build` → `vite build`, `dev` → `vite build --watch`, `test` → `vitest run`. Test invocation pattern: `npm run test --prefix packages/ui` from repo root, or `npm test` from `packages/ui/`.
- **No standalone `vitest.config.*` file** in `packages/ui/` — vitest uses defaults plus implicit jsdom env from the dependency. Phase 1.A's jest-axe integration may need a `vitest.config.ts` added with `test.setupFiles` to register the matcher.
- **`dashboard/package.json`** scripts: `dev` → `vite`, `build` → `tsc -b && VITE_BASE_PATH=/dashboard/ vite build`. No `test` script in the head of the scripts block (verify at implementation if dashboard tests exist elsewhere).
- **No `.eslintrc*` or `eslint.config*` files** in either `dashboard/` or `packages/ui/` root. No `eslint-plugin-jsx-a11y` configured. ESLint tooling decision is out of this PR's scope (filed as Phase 6 follow-up).
- **CI workflows** at `.github/workflows/` (verify at implementation): no existing a11y job. New CI gate is greenfield.

### Existing primitives — concrete file enumeration (verified)

`ls packages/ui/src/components/` at HEAD `48e52c83` returns 12 `.tsx` files plus 1 `.test.tsx` file:

```
AgentFilter.tsx
CopyButton.tsx
EmptyState.tsx
ErrorState.tsx
ListRow.tsx
LoadingState.tsx
MarkdownContent.tsx
Pagination.tsx
SelectFilter.tsx
StatusBadge.tsx
TaskStatusBadge.tsx
TimeRangeFilter.tsx
TokenBudgetBar.tsx
TokenBudgetBar.test.tsx   ← only existing test file
```

**Critical scope discovery: 11 of 12 primitives have NO test file.** Phase 1.A's "add an axe assertion alongside existing tests" is misframed in the original Phase 1.A description — it actually means **creating the first test file for 11 primitives, then adding the axe assertion as part of that initial test file**. Each new test file has at minimum: render-the-default-props test + axe assertion. Approximate scope: ~20-40 new lines per file × 11 files = 220-440 new lines of test code in Phase 1.A.

This changes the work shape but not the bound — Phase 1.A is still a contained workstream, just larger than initially framed. The 50-line halt threshold on backend work in Phases 2.B/2.C of mika#667 doesn't apply here (this is test code, not API/SQL); a separate halt threshold for test-file-count is unnecessary because each new test file is bounded by the primitive's existing surface.

**Worktree drift note:** PR #990 (mika#662, merged 2026-05-06 09:00:48Z) added `LiveRefreshToggle.tsx`. My worktree branched at HEAD `48e52c83` before PR #990's merge. At dispatch time, the worktree must `git fetch origin main && git rebase origin/main`, after which the primitive count becomes 13 (LiveRefreshToggle's test file is also a Phase 1.A target if PR #990 didn't already include it — verify at rebase).

### Existing primitives (a11y-relevant per-component notes)

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

## Phase 2 — Triage findings (mechanical rules)

For each finding from Phase 1, assign one of three dispositions using the mechanical rules below. **No judgment-dependent classifications.** The audit doc's disposition column is the durable record.

### Disposition rules (in priority order)

**Rule 1 — `accept-with-rationale`:** the finding is intentional or out-of-scope (e.g., a known browser quirk, a third-party library limitation, a design-system decision Vincent owns). Requires a one-line rationale documented in the disposition column. Mechanical check: if the rationale is missing, this disposition is rejected.

**Rule 2 — `fix-here` candidate:** the finding's source path is **under `packages/ui/src/components/`** AND the finding is not in the always-file-follow-up exclusion list below. Mechanical check: `path.startsWith('packages/ui/src/components/')` returns true.

**Rule 3 — `file-follow-up`:** any finding not matching rules 1 or 2. This includes:
- Findings with source paths under `dashboard/src/` (any subdirectory) — page-specific.
- Findings under `packages/ui/src/theme.css` (color tokens) — design-system territory, owned by Vincent.
- Findings in third-party rendered output (e.g., `react-markdown` output) where the fix requires a wrapper component, not a primitive change.

### Always-file-follow-up exclusion list (regardless of source path)

The following finding categories are **always** `file-follow-up`, even if they originate in `packages/ui/src/components/`:

- **Modal/drawer focus-trap.** The fix requires designing a new `<Modal>` or `<Drawer>` primitive (with focus-lock, ESC dismissal, scroll-lock). New-primitive design is out of this PR's audit-and-fix scope. Filed as a follow-up regardless of severity.
- **New primitive needed.** Any finding whose fix requires creating a new primitive (rather than modifying an existing one). The audit catalogues; the new primitive is its own ticket.
- **API surface change required.** Any finding whose fix requires changing a primitive's prop signature in a way that breaks consumers. These are filed as deprecation/migration tickets, not in-PR fixes.

### Halt threshold (architect F3 calibration note)

If Phase 1 surfaces more than **15 `fix-here` candidates** (after Rule 1 and the always-follow-up exclusions are applied), the plan halts and surfaces to operator. The 15-number calibration:

- 12-13 primitives × 1 axe assertion each = 12-13 baseline `fix-here` items (the axe-pass-or-fix work).
- Each axe failure typically surfaces 1-2 specific ARIA/keyboard fixes per primitive → +12-26 items if every primitive has 1-2 violations.
- Realistic median: 12-13 baseline + ~3 ARIA fixes (the worst offenders) = ~15-18 fix-here candidates.

A count significantly above 15 (say 25+) signals that primitives have systemic gaps requiring sustained remediation work — better as its own ticket. A count below 15 is the natural "audit + light fix" envelope. The threshold is a soft signal; the implementer surfaces and operator decides.

Above-threshold split: ship audit + CI gate + a small subset of fix-here items in this PR; file the remainder as a "primitive a11y remediation sprint" ticket targeting milestone#13 or its successor.

## Phase 3 — Apply fix-here remediations

**Files touched (mechanical scope):** files matching the glob `packages/ui/src/components/*.tsx` (and their accompanying `*.test.tsx`). **No file outside this glob may be modified in Phase 3.** Findings from outside this glob are `file-follow-up` per Phase 2's mechanical rule, regardless of severity.

**Concrete file inventory at Phase 3 entry** (per Phase 0 pin, post-rebase to pick up PR #990):

```
packages/ui/src/components/
  ├── AgentFilter.tsx
  ├── CopyButton.tsx
  ├── EmptyState.tsx
  ├── ErrorState.tsx
  ├── ListRow.tsx
  ├── LiveRefreshToggle.tsx       (added by PR #990)
  ├── LoadingState.tsx
  ├── MarkdownContent.tsx
  ├── Pagination.tsx
  ├── SelectFilter.tsx
  ├── StatusBadge.tsx
  ├── TaskStatusBadge.tsx
  ├── TimeRangeFilter.tsx
  ├── TokenBudgetBar.tsx
  └── (test files alongside, one per primitive after Phase 1.A creates them)
```

**Files explicitly out of bounds for Phase 3:**

- `packages/ui/src/index.ts` — re-exports only, no a11y surface.
- `packages/ui/src/theme.css` — design system tokens, Vincent-owned.
- `packages/ui/src/utils/` — utility functions, not a11y-relevant.
- Any file under `dashboard/src/`.
- Any file under `crates/`.
- `.github/workflows/` — CI is Phase 4's surface, not Phase 3's.

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
