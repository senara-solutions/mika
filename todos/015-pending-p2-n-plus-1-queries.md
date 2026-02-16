---
status: pending
priority: p2
issue_id: "015"
tags: [code-review, performance]
dependencies: []
---

# N+1 Queries in Follow-ups and WhatsApp Handler

## Problem Statement

Follow-up tasks and WhatsApp message handler query users in a loop, issuing separate DB queries for each user instead of batch loading.

## Findings

- **Source:** Performance Oracle (CRITICAL-5), Python Code Quality
- **Locations:**
  - `app/worker/tasks/follow_ups.py`
  - `app/channels/whatsapp/handlers.py`

## Proposed Solutions

### Option A: Use eager loading / batch queries (Recommended)
- Use `selectinload()` or `joinedload()` for relationships
- Batch user lookups with `WHERE IN` clauses
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] No N+1 query patterns in follow-up tasks
- [ ] No N+1 query patterns in WhatsApp handler
- [ ] Query count is constant regardless of result set size

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
