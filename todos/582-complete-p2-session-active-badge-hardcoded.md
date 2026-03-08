---
status: pending
priority: p2
issue_id: "582"
tags: [code-review, quality]
dependencies: []
---

# Session "Active" Badge is Hardcoded

## Problem Statement
In `SessionDetail.tsx`, every session shows an "Active" badge regardless of whether the session is actually active. Completed sessions with an `ended_at` timestamp incorrectly display as "Active".

## Findings
- **Source:** TypeScript Reviewer
- **Location:** `dashboard/src/pages/SessionDetail.tsx` lines 89-91

## Proposed Solutions
Check `session.ended_at === null` to determine active/completed status.

## Acceptance Criteria
- [ ] Active badge only shows when session.ended_at is null
- [ ] Completed sessions show a different indicator

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | TypeScript Reviewer found hardcoded badge |

## Resources
- PR #89
