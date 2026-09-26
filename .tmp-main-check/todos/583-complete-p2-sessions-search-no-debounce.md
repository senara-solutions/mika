---
status: pending
priority: p2
issue_id: "583"
tags: [code-review, performance]
dependencies: []
---

# Sessions Agent Search Fires API Call on Every Keystroke

## Problem Statement
In `Sessions.tsx`, the agent search input calls `updateFilter('agent_id', e.target.value)` on every `onChange` event, updating URL params and triggering a new API query per keystroke. This hammers the API unnecessarily.

## Findings
- **Source:** TypeScript Reviewer
- **Location:** `dashboard/src/pages/Sessions.tsx` line 78

## Proposed Solutions
Either debounce the input (300ms) or use the same pattern as Timeline (local state + search button).

## Acceptance Criteria
- [ ] Agent search does not fire a query on every keystroke
- [ ] Search still works when user types and pauses/submits

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | TypeScript Reviewer found keystroke query spam |

## Resources
- PR #89
