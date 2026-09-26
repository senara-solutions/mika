---
status: pending
priority: p3
issue_id: "587"
tags: [code-review, quality]
dependencies: []
---

# Remove Dead UI: Export Buttons and Settings Placeholder

## Problem Statement
Three pages have non-functional Export buttons with no onClick handlers, and SettingsPage.tsx is a pure placeholder ("Settings coming soon"). These are YAGNI violations that mislead users.

## Findings
- **Source:** Code Simplicity Reviewer, TypeScript Reviewer
- **Locations:** Export buttons in `Timeline.tsx`, `Sessions.tsx`, `SessionDetail.tsx`. Placeholder `SettingsPage.tsx` + route + sidebar entry.

## Proposed Solutions
Remove Export buttons and SettingsPage entirely. Add back when functionality exists.

## Acceptance Criteria
- [ ] No non-functional buttons in UI
- [ ] Settings page and route removed

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Simplicity + TS reviewers flagged dead UI |

## Resources
- PR #89
