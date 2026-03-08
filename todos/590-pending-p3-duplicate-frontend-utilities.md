---
status: pending
priority: p3
issue_id: "590"
tags: [code-review, quality]
dependencies: []
---

# Duplicate Frontend Utilities

## Problem Statement
Several utility patterns are duplicated across pages:
- `eventTypeBadge` in Timeline.tsx and TraceDetail.tsx
- `updateFilter` / `setPage` in Timeline.tsx and Sessions.tsx
- `useFormatTime.ts` named as a hook but contains plain functions

## Findings
- **Source:** TypeScript Reviewer
- Extract shared `eventTypeBadge` to a utility module
- Extract `useSearchParamsFilters` hook
- Rename `useFormatTime.ts` to `formatTime.ts` or move to `utils/`

## Acceptance Criteria
- [ ] No duplicated utility functions across pages
- [ ] File naming follows React conventions (use prefix = hooks only)

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | TypeScript Reviewer found duplications |

## Resources
- PR #89
