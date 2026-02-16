---
status: pending
priority: p2
issue_id: "009"
tags: [code-review, data-integrity, architecture]
dependencies: []
---

# Cross-Store Deletion is Not Atomic

## Problem Statement

The privacy delete endpoint cascades deletion across Neo4j -> Postgres -> Redis sequentially. If any step fails mid-way, the user ends up in a partially deleted state with no rollback mechanism.

**Why it matters:** Data inconsistency; partial deletion may leave orphaned data or prevent re-deletion.

## Findings

- **Source:** Data Integrity Guardian (CRITICAL), Architecture Strategist
- **Location:** `app/api/routes/privacy.py` — `privacy_delete()` endpoint
- **Evidence:** Three separate delete calls with no transaction coordination or compensation logic

## Proposed Solutions

### Option A: Soft-delete with background cleanup (Recommended)
- Mark user as `deleted` in Postgres immediately
- Queue background Celery task to clean up Neo4j and Redis
- Retry failed steps; idempotent operations
- **Pros:** User gets immediate feedback; eventual consistency; retryable
- **Cons:** Data exists briefly after "deletion"
- **Effort:** Medium
- **Risk:** Low

### Option B: Best-effort with error logging
- Continue current approach but catch individual errors, log them, and continue
- Return partial success status to user
- **Pros:** Simple; user data mostly deleted
- **Cons:** May leave orphaned data
- **Effort:** Small
- **Risk:** Medium

## Recommended Action
<!-- Filled during triage -->

## Technical Details

**Affected files:**
- `app/api/routes/privacy.py`
- Potentially new Celery task for async cleanup

## Acceptance Criteria

- [ ] Deletion handles partial failures gracefully
- [ ] Failed deletions can be retried
- [ ] User is informed of deletion status
- [ ] No orphaned data remains permanently

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | Cross-store transactions are inherently non-atomic |

## Resources

- Saga pattern for distributed transactions
