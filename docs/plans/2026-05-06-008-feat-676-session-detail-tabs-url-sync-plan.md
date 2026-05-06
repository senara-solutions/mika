---
title: "feat: Sync session detail tabs with URL path segments"
type: feat
status: active
date: 2026-05-06
---

# feat: Sync session detail tabs with URL path segments

## Overview

Change session detail tab navigation from query-param encoding (`?tab=llm-calls`) to path-segment encoding (`/sessions/:id/llm-calls`). This makes tab URLs bookmarkable, shareable, and consistent with REST conventions.

## Problem Frame

Session detail page tabs (Messages, LLM Calls, Tool Calls, Skills) currently use `?tab=` query params. While functional, path segments are more natural for primary navigation tabs — they're easier to read, share, and bookmark. The issue requests URLs like `/dashboard/sessions/<id>/messages`.

## Requirements Trace

- R1. Clicking a tab updates the URL path segment (not query param)
- R2. Loading a URL with a tab segment selects that tab
- R3. Loading `/sessions/:id` (no tab segment) defaults to Messages
- R4. Invalid tab segments fall back to Messages
- R5. Session list links continue to work (default to messages tab)
- R6. Sub-tab pagination resets on tab switch
- R7. No backend changes required (SPA fallback already handles nested paths)

## Scope Boundaries

- AgentDetail tabs remain on `?tab=` — separate concern, not part of this issue
- Pagination within tabs stays as local state (not promoted to URL params)
- No new shared Tab component — the existing hand-rolled tab bar works fine

## Context & Research

### Relevant Code and Patterns

- `dashboard/src/App.tsx` — route definitions, currently `sessions/:sessionId` (line 35)
- `dashboard/src/pages/SessionDetail.tsx` — tab state via `useSearchParams` (lines 396-413), tab bar UI (lines 810-834)
- `dashboard/src/pages/Sessions.tsx` — links to `/sessions/${s.id}` (line 157)
- `crates/mika-agent/src/server/embedded_dashboard.rs` — SPA fallback serves index.html for all unmatched paths (line 125), so `/sessions/:id/messages` works without server changes
- React Router v7.6.3 supports optional params with `?` suffix (e.g., `:tab?`)

### Institutional Learnings

- `docs/solutions/best-practices/dashboard-url-state-shareable-links-pattern-2026-05-06.md` — documents the existing `?tab=` pattern with type-guard validation and default-tab omission
- `docs/solutions/architecture-patterns/embed-dashboard-spa-rust-embed.md` — confirms SPA fallback handles arbitrary sub-paths
- `docs/solutions/best-practices/dashboard-live-refresh-consistency-pattern-2026-05-06.md` — `isDefaultView` guard must account for URL-reflected state; SessionDetail uses status-based auto-refresh (not `isDefaultView`), so no change needed

## Key Technical Decisions

- **Optional param over nested routes:** `sessions/:sessionId/:tab?` is simpler than parent/child route nesting. Keeps `SessionDetail` as a single route component. React Router v7 supports `?` suffix for optional params.
- **No redirect for missing tab:** `/sessions/:id` renders Messages directly rather than redirecting to `/sessions/:id/messages`. This preserves existing links and avoids unnecessary navigation.
- **`useNavigate` with `replace: true`:** Tab switches use `navigate(path, { replace: true })` to avoid polluting browser history — users expect Back to leave the page, not cycle through tabs.
- **Preserve query params on tab switch:** Any existing query params (future pagination) carry forward by appending `search` to the navigate call.

## Implementation Units

- [ ] **Unit 1: Route definition and tab state from path params**

**Goal:** Change routing from `sessions/:sessionId` to `sessions/:sessionId/:tab?` and switch tab state from `useSearchParams` to `useParams`.

**Requirements:** R1, R2, R3, R4, R6

**Dependencies:** None

**Files:**
- Modify: `dashboard/src/App.tsx`
- Modify: `dashboard/src/pages/SessionDetail.tsx`

**Approach:**
- In `App.tsx`, change the route path from `sessions/:sessionId` to `sessions/:sessionId/:tab?`
- In `SessionDetail.tsx`, replace `useSearchParams`-based tab logic with `useParams<{ sessionId: string; tab?: string }>()`. Validate the `tab` param with the existing `isSessionTab()` type guard; default to `'messages'` when absent or invalid
- Replace `setActiveTab` to use `useNavigate`: build the new path as `/sessions/${sessionId}/${tab}` (omit tab segment for `'messages'` to keep URL clean, or include it — either works since the route handles both). Use `replace: true` to avoid history pollution
- Preserve sub-tab pagination reset on tab switch (existing behavior)
- Remove `useSearchParams` import if no longer needed (check if any other state uses it)

**Patterns to follow:**
- Existing `isSessionTab()` type guard and `VALID_SESSION_TABS` array (lines 314-318)
- Existing pagination reset pattern (lines 409-412)

**Test scenarios:**
- Happy path: navigating to `/sessions/abc/llm-calls` renders the LLM Calls tab
- Happy path: navigating to `/sessions/abc/messages` renders the Messages tab
- Happy path: navigating to `/sessions/abc` (no tab segment) renders the Messages tab
- Edge case: navigating to `/sessions/abc/invalid-tab` falls back to Messages tab
- Happy path: clicking a tab button updates the URL path to include the tab segment
- Happy path: switching tabs resets sub-tab pagination to page 1

**Verification:**
- Tab state is driven by URL path param, not query param
- Browser back/forward navigates between tabs correctly (with `replace: true`, back should leave the page)
- All four tabs render their content when accessed via direct URL

- [ ] **Unit 2: Update inbound links and documentation**

**Goal:** Ensure session list links work with the new routing and update documentation.

**Requirements:** R5

**Dependencies:** Unit 1

**Files:**
- Modify: `dashboard/src/pages/Sessions.tsx`
- Modify: `dashboard/CLAUDE.md`

**Approach:**
- Session list links at `Sessions.tsx` line 157 currently use `/sessions/${s.id}`. These already work because the optional `:tab?` param defaults to messages. Optionally update to `/sessions/${s.id}/messages` for explicitness — either way is correct
- Check `AgentDetail.tsx` for any links to session detail (line 520) — same consideration
- Update `dashboard/CLAUDE.md` line 29 to document that SessionDetail now uses path segments instead of `?tab=`. Keep the AgentDetail `?tab=` mention unchanged

**Patterns to follow:**
- Existing `<Link to={...}>` patterns in list pages

**Test scenarios:**
- Happy path: clicking a session in the list navigates to the session detail with Messages tab active
- Happy path: any cross-page links to session detail land on the correct default tab

**Verification:**
- All navigation paths to session detail pages work correctly
- Documentation accurately reflects the new URL pattern

## System-Wide Impact

- **Interaction graph:** Only the session detail page is affected. The SPA fallback handler, session list page, and agent detail page (links to sessions) are adjacent but require minimal or no changes.
- **API surface parity:** AgentDetail tabs intentionally remain on `?tab=` — this is noted as a separate concern. If consistency is desired later, it would be a follow-up to mika#664.
- **Unchanged invariants:** All API endpoints, data fetching hooks, and backend routing remain unchanged. The `useSearchParamsFilter` hook (used by list pages) is unaffected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Existing bookmarks with `?tab=` break | Low risk — feature is new, unlikely anyone has bookmarked `?tab=` URLs. Could add a one-time redirect from `?tab=` to path segment, but YAGNI for now. |
| Browser history pollution from tab clicks | Using `navigate(path, { replace: true })` prevents this |

## Sources & References

- Related issue: #676
- Related cross-cutting issue: #664 (URL state management)
- React Router v7 optional params: supported via `?` suffix in route path
