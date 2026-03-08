---
status: pending
priority: p1
issue_id: "576"
tags: [code-review, quality]
dependencies: []
---

# Auto-Refresh Toggle Disables Entire Timeline Query

## Problem Statement
The `autoRefresh` state in `Timeline.tsx` is passed as the `enabled` parameter to `useTimeline`, which disables the entire query when toggled off. The user expects toggling auto-refresh to stop polling, not to blank out the data. Additionally, the LIVE badge is unconditionally shown regardless of the toggle state.

## Findings
- **Source:** TypeScript Reviewer agent
- **Severity:** HIGH — functional bug, toggling off makes all data disappear
- **Location:** `dashboard/src/pages/Timeline.tsx` line 36 (`useTimeline(filters, autoRefresh)`), `dashboard/src/api/timeline.ts` line 38 (`refetchInterval` controlled by `isDefaultView` not by the toggle)
- The `enabled` parameter and `refetchInterval` are conflated
- LIVE badge at line 67-70 is unconditional

## Proposed Solutions

### Option A: Pass autoRefresh to control refetchInterval only
- Change `useTimeline` signature to accept a `refetchInterval` override
- Always keep `enabled: true`, use `autoRefresh ? 5000 : false` for refetch
- Conditionally show LIVE badge based on autoRefresh state
- **Effort:** Small
- **Risk:** Low

## Technical Details
- **Affected files:** `dashboard/src/pages/Timeline.tsx`, `dashboard/src/api/timeline.ts`

## Acceptance Criteria
- [ ] Toggling auto-refresh off stops polling but keeps current data visible
- [ ] LIVE badge only shows when auto-refresh is on
- [ ] Toggling auto-refresh on resumes polling

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | TypeScript reviewer found toggle/enabled conflation |

## Resources
- PR #89: feat: Observability Dashboard (MVP)
