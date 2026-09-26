---
status: pending
priority: p3
issue_id: "589"
tags: [code-review, quality]
dependencies: []
---

# Dashboard Accessibility (a11y) Improvements

## Problem Statement
Multiple accessibility issues across the dashboard: auto-refresh toggle lacks role="switch" and aria-checked, table headers missing scope="col", pagination/back buttons lack aria-labels, search inputs lack labels.

## Findings
- **Source:** TypeScript Reviewer
- **Locations:** `Timeline.tsx` (toggle), `Pagination.tsx` (buttons), all table views (th elements), search inputs throughout

## Proposed Solutions
Add ARIA attributes: role="switch" + aria-checked on toggle, scope="col" on th, aria-label on icon-only buttons, aria-label on search inputs, aria-hidden on decorative dots.

## Acceptance Criteria
- [ ] Toggle has role="switch" and aria-checked
- [ ] All icon-only buttons have aria-label
- [ ] Table headers have scope="col"

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | TypeScript Reviewer flagged 5 a11y issues |

## Resources
- PR #89
