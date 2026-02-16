---
status: pending
priority: p1
issue_id: "006"
tags: [code-review, architecture, python]
dependencies: []
---

# Deprecated asyncio.get_event_loop().run_until_complete() Pattern

## Problem Statement

Celery worker tasks use `asyncio.get_event_loop().run_until_complete()` to call async functions from sync context. This is deprecated in Python 3.12+ and will raise `DeprecationWarning` or fail. It also risks reusing a closed or running event loop.

**Why it matters:** Celery tasks will break on Python 3.12+; potential runtime errors.

## Findings

- **Source:** All 7 review agents identified this issue
- **Locations:**
  - `app/worker/tasks/briefings.py`
  - `app/worker/tasks/follow_ups.py`
- **Evidence:** `asyncio.get_event_loop().run_until_complete(...)` pattern used in both files

## Proposed Solutions

### Option A: Use `asyncio.run()` (Recommended)
- Replace `asyncio.get_event_loop().run_until_complete(coro)` with `asyncio.run(coro)`
- **Pros:** Python 3.12+ compatible; creates fresh event loop; simple
- **Cons:** Creates new event loop each call (acceptable for Celery tasks)
- **Effort:** Small
- **Risk:** Low

### Option B: Use `asgiref.sync_to_async` / custom bridge
- Use a library to manage the async/sync boundary
- **Pros:** More sophisticated event loop management
- **Cons:** Adds dependency; overkill for Celery tasks
- **Effort:** Medium
- **Risk:** Low

## Recommended Action
<!-- Filled during triage -->

## Technical Details

**Affected files:**
- `app/worker/tasks/briefings.py`
- `app/worker/tasks/follow_ups.py`

## Acceptance Criteria

- [ ] No usage of `asyncio.get_event_loop().run_until_complete()` in codebase
- [ ] Celery tasks use `asyncio.run()` or equivalent modern pattern
- [ ] All worker task tests pass
- [ ] No deprecation warnings on Python 3.12+

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | Consensus across all 7 agents |

## Resources

- Python 3.12 asyncio changes
