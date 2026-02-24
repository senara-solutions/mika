---
status: ready
priority: p2
issue_id: "098"
tags: [code-review, quality, testing, data-integrity]
dependencies: []
---

# Add unit tests for compact_old_memory_events

## Problem Statement
The `compact_old_memory_events()` function is ~120 lines of complex logic (SELECT, aggregate, INSERT, DELETE across two tables) with zero unit tests. This is the most complex new function in the PR and the most likely to contain bugs. Multiple review agents flagged this gap.

## Findings
- File: `crates/mika-agent/src/db.rs` (compact_old_memory_events function)
- Function performs: SELECT old events → GROUP BY month → INSERT summaries → DELETE originals
- No test coverage for any path: happy path, empty set, boundary dates, idempotency
- Flagged by: Data Integrity Guardian (HIGH), Architecture Strategist, Code Simplicity Reviewer

## Proposed Solutions

### Option 1: Comprehensive test suite (Recommended)
Add tests covering:
- Happy path: events older than retention_days get compacted into monthly summaries
- Empty set: no events older than threshold → no-op
- Boundary: events exactly at threshold boundary
- Idempotency: running twice produces same result (no duplicate summaries)
- Partial months: events spanning month boundaries
**Effort:** Medium
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria
- [ ] Happy path test passes
- [ ] Empty set test passes
- [ ] Boundary date test passes
- [ ] Idempotency test passes
- [ ] All existing tests still pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Multiple agents identified missing test coverage for complex data transformation
