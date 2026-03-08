---
status: pending
priority: p3
issue_id: "591"
tags: [code-review, quality]
dependencies: []
---

# Missing 404 / Catch-All Route

## Problem Statement
`App.tsx` has no wildcard route. Navigating to an unknown path renders the Layout with empty content area.

## Findings
- **Source:** TypeScript Reviewer
- **Location:** `dashboard/src/App.tsx`

## Proposed Solutions
Add `<Route path="*" element={<NotFound />} />` with a simple not-found page.

## Acceptance Criteria
- [ ] Unknown paths show a helpful "Page not found" message

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | TypeScript Reviewer flagged missing catch-all |

## Resources
- PR #89
