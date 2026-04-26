---
title: "feat(ui): dashboard copy feedback — visual confirmation on copy-to-clipboard actions"
type: feat
status: active
date: 2026-04-26
origin: senara-solutions/mika#665
---

# Plan — dashboard copy feedback (mika#665)

**Issue:** [mika#665](https://github.com/senara-solutions/issues/665) — `Dashboard > Copy feedback: visual confirmation on copy-to-clipboard actions`
**Branch:** `feat/665/dashboard-copy-feedback-confirmation`
**Type:** feat (Phase 2 primitive in milestone #13)
**Labels:** enhancement, dashboard

## Problem

Copy icons appear next to IDs and content across multiple dashboard pages. Clicking them copies content to the clipboard but provides minimal visible confirmation — user clicks, doesn't notice the change, tries again (and may trigger double paste elsewhere).

## Discovery (verified during planning)

Audit answer for the issue body's "Does it provide feedback today and it's just hidden, or does it genuinely have none?":

**`<CopyButton />` already provides feedback** (verified at `packages/ui/src/components/CopyButton.tsx:13,19-20,32`):
- `useState(copied: boolean)` tracks the copy state.
- After `await navigator.clipboard.writeText(text)`, sets `copied=true` for 2000ms.
- Icon swap: `<Copy size={12} />` → `<Check size={12} className="text-emerald-400" />`.

**Gaps in the existing feedback:**
1. Icon swap is abrupt — no CSS transition on the icon itself. Only the button has `transition-opacity` for hover (line 29). Per rulebook §5 ("Soft Minimalism", transition states), the `Copy → Check` swap should have a smooth transition.
2. **Hand-rolled clipboard call exists** at `dashboard/src/pages/TraceDetail.tsx` (verified via grep — duplicates the exact pattern of CopyButton, ~5 lines of `useState(copied)` + `navigator.clipboard.writeText` + 2s timeout). Should be migrated to `<CopyButton />`.
3. **`<TraceIdWidget />` does NOT exist yet** — per `docs/design/dashboard-stitch-map.md` Phase 2, this primitive will be built from mika#652 and mika#653 work. Out of scope for #665. The acceptance criterion "`<TraceIdWidget />` uses `<CopyButton />` internally" is forward-looking — when TraceIdWidget is built later, it must consume `<CopyButton />`. We don't build TraceIdWidget here; we just ensure the prerequisite (CopyButton) is solid.

## Approach

### Change 1 — Smooth the icon-swap transition in `<CopyButton />`

**File:** `mika/packages/ui/src/components/CopyButton.tsx`

Wrap the icon swap in a transition (CSS opacity + scale fade, ~150ms). Keep the existing `Copy/Check` swap and the 2000ms timeout. Per rulebook §5: "no harsh notifications" — the icon transition is the soft visual confirmation pattern.

Suggested implementation (final form is implementer's call within the rulebook envelope):

```tsx
<button onClick={handleCopy} className={...} title={title}>
  <span className="relative inline-flex items-center justify-center">
    <Copy
      size={12}
      className={`transition-opacity duration-150 ${copied ? 'opacity-0' : 'opacity-100'}`}
    />
    <Check
      size={12}
      className={`absolute transition-opacity duration-150 text-emerald-400 ${copied ? 'opacity-100' : 'opacity-0'}`}
    />
  </span>
</button>
```

**Use pure CSS opacity transition. Do NOT add framer-motion.** Per architect Finding 1 (session `f9191e17-...`): `framer-motion` is not in either `packages/ui/package.json` or `dashboard/package.json` (verified `grep -c "framer-motion"` returns 0 in both). Even if it were available, KISS applies — framer-motion is appropriate for layout animations, not 2-element icon crossfades (review-guide.md § 6 + KISS).

**Use `data-testid` attributes** on the button and both icon containers per architect Finding 3:
- `data-testid="copy-button"` on the button element.
- `data-testid="copy-icon"` on the `Copy` icon container.
- `data-testid="check-icon"` on the `Check` icon container.

This removes test fragility — SVG-presence or button-role queries break if `lucide-react` swaps icons. Shared component library convention is to ship `data-testid` for stable consumer test handles. Cite: test-stability over implementation-shape (review-guide.md § 6).

**No toast.** Issue body says "optional toast" — YAGNI for v1. The icon swap is the established pattern across `@senara-solutions/ui`. A toast adds notification surface (positioning, dismissal, ARIA-live region) for marginal value when the click happens at the affordance itself. If operator demand surfaces post-merge, file a follow-up.

**Rapid-click edge case:** the absolute-overlay shape is preferred over `key={copied}` remount (architect Finding 2 second-pass — implementer-discretion within envelope). Absolute-overlay is more predictable under rapid clicks because there's no animation-interrupt edge case; the `setTimeout` simply re-fires and the opacity stays at 1.0 throughout. `key={copied}` remount triggers React reconciliation on each toggle and risks animation-interrupt visual artifacts under rapid clicks. Implementer can deviate within rulebook envelope, but absolute-overlay is the recommended default.

### Change 2 — Migrate `dashboard/src/pages/TraceDetail.tsx` hand-rolled clipboard

**File:** `mika/dashboard/src/pages/TraceDetail.tsx`

Replace the hand-rolled `navigator.clipboard.writeText` + `useState(copied)` + setTimeout with `<CopyButton text={...} />` from `@senara-solutions/ui`. Pre-edit discovery: read the surrounding context to confirm the hand-rolled instance is purely a copy-button (not, e.g., a copy-action embedded in some other UI). Verified via grep that the hand-rolled pattern is identical to CopyButton's own implementation — straightforward migration.

### Change 3 — Audit grep for any other hand-rolled callsites

**Command:** `grep -rln "navigator.clipboard" mika/dashboard/src/ mika/packages/ui/src/`

Two callsites today: `CopyButton.tsx` itself (the canonical), and `TraceDetail.tsx` (the hand-rolled one to migrate). If any new callsites appear during implementation (e.g., from in-flight work on other branches), migrate them too. AC closes the gap explicitly.

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `packages/ui/src/components/CopyButton.tsx` | +5 lines: icon-swap transition (CSS opacity/scale on Copy and Check icons) |
| 2 | `dashboard/src/pages/TraceDetail.tsx` | -10/+3: replace hand-rolled `useState(copied)` + clipboard write with `<CopyButton text={...} />` import + JSX |

Net diff: ~15 lines changed, 2 files. Smallest grooming session of the night.

## Tests

Inline in `packages/ui/src/components/CopyButton.test.tsx` (new test file or existing — verify presence during implementation). Tests query by `data-testid` attributes per architect Finding 3 — stable across icon-library changes.

1. **`renders Copy icon visible by default`** — render `<CopyButton text="abc" />`, assert element with `data-testid="copy-icon"` has `opacity-100`, `data-testid="check-icon"` has `opacity-0`.
2. **`crossfades to Check icon after click`** — mock `navigator.clipboard.writeText`, click `data-testid="copy-button"`, assert `data-testid="check-icon"` transitions to `opacity-100`, `data-testid="copy-icon"` to `opacity-0`.
3. **`reverts to Copy icon after timeout`** — same setup, advance fake timers by 2100ms, assert opacities revert (`copy-icon` to `opacity-100`, `check-icon` to `opacity-0`).
4. **`calls clipboard.writeText with the text prop`** — mock clipboard, click `data-testid="copy-button"`, assert called with the exact `text` prop value.
5. **`stops event propagation`** — render inside a parent with `onClick` handler, click `data-testid="copy-button"`, assert parent handler NOT called.

These tests verify the canonical primitive's contract; downstream consumers don't need their own copy tests after migration.

## Acceptance criteria

- [ ] `<CopyButton />` icon swap uses a smooth CSS transition (opacity-based, ~150ms) per rulebook §5 — no hard cut.
- [ ] `dashboard/src/pages/TraceDetail.tsx` uses `<CopyButton />` instead of hand-rolled `navigator.clipboard.writeText`.
- [ ] `grep -rn "navigator.clipboard" mika/dashboard/src/ mika/packages/ui/src/` returns only the `CopyButton.tsx` callsite (the canonical implementation). Any other hits indicate a missed migration.
- [ ] All 5 tests above pass.
- [ ] `npm run build --prefix dashboard` clean.
- [ ] No existing test regressions (`cargo test` and dashboard test suite).

## Out of scope

- Toast notifications. Issue body marks them "optional" — defer until operator demand surfaces post-merge.
- `<TraceIdWidget />` migration. The widget doesn't exist yet — built later by mika#652 / mika#653 work per dashboard-stitch-map. The forward-looking acceptance criterion "`<TraceIdWidget />` uses `<CopyButton />` internally" is preserved as a constraint for the implementer of those tickets, not for this one.
- Migrating CopyButton's caller convention (e.g., adding new props like `onCopySuccess` callback) — YAGNI; current API is sufficient for the dashboard's needs.

## Forward constraint for #652/#653 (per architect Finding 2)

Post-grooming, add a comment to mika#665's issue body or a follow-up note that surfaces the forward constraint to mika#652 and mika#653 implementers: **"`<TraceIdWidget />` (built by mika#652/#653) MUST use `<CopyButton />` internally."** The constraint lives in the issue body where milestone tooling (and future implementers) will surface it. Cite: issue-as-versioned-contract pattern (mika-platform#52 body-edit convention).

## Risks

| Risk | Mitigation |
|---|---|
| Icon-swap transition introduces visual flicker on slow devices | 150ms is a standard fast-feedback duration; tested across modern browsers in development. If flicker reported post-merge, reduce to 100ms or revert to instant swap. |
| `<CopyButton />` API change breaks existing dashboard callers | API unchanged: same `text` and `className` props. Visual output changes (smooth transition vs hard cut), no consumer-side change required. |
| Hidden third hand-rolled callsite I missed in grep | AC #3 (post-implementation grep) catches any missed callsite. |
| `framer-motion` dependency conflict if implementer picks that route | The plan's suggested shape uses pure CSS transitions — no new dependency. If the implementer prefers `framer-motion`, verify it's already in `package.json` (not in our current dependency tree per implementation memory) before committing to that route. |

## Sequencing

1. **Change 1 first** (CopyButton transition refinement). Small, contained.
2. **Change 2 second** (TraceDetail.tsx migration). Builds on Change 1's smoothed CopyButton.
3. **Tests inline.**
4. **Run** dashboard test suite + `npm run build`.
5. **Open PR** cross-referencing #665. PR description describes Change 1 as primitive refinement and Change 2 as dashboard cleanup — small, easy ship per issue body.

## Verification

End-to-end manual test:

1. Run dashboard dev server: `VITE_MIKA_DASHBOARD_TOKEN=<token> npm run dev:dashboard`.
2. Navigate to any page with a copy button (Agent detail section title, Team Runs run ID, Trace detail).
3. Click the copy button. Observe: smooth icon transition from Copy → Check → Copy (~2s on, then fade back).
4. Verify TraceDetail-specific copy buttons behave identically to other pages.
5. Inspect DOM: confirm the `<CopyButton />` component is in use everywhere (no inline `useState(copied)` patterns).

## Discovery items (resolved during planning)

1. **`<CopyButton />` already has feedback** — `packages/ui/src/components/CopyButton.tsx:13,19-20,32`. The audit asked by the issue body resolves to "feedback exists; refine the transition shape, don't add a new mechanism."
2. **One hand-rolled callsite to migrate** — `dashboard/src/pages/TraceDetail.tsx`. Verified via grep across `dashboard/src/` and `packages/ui/src/`.
3. **`<TraceIdWidget />` does not exist** — per dashboard-stitch-map.md Phase 2 inventory, the widget is "build new" pending mika#652/#653. Forward-looking constraint preserved for those tickets, not this one.
4. **No `framer-motion` in current dependency tree** — implementer should use pure CSS transitions to avoid adding a dependency for a 150ms fade.
