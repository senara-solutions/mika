---
status: pending
priority: p2
issue_id: "629"
tags: [code-review, quality, rewind]
dependencies: []
---

# Collapse duplicate rewind exchange helper functions

## Problem Statement

`find_exchanges_in_messages` and `find_exchanges_in_messages_cross_session` in `rewind.rs` duplicate ~80% of their logic. The cross-session variant correctly handles single-session input (it locks to the first session_id encountered), making the single-session helper redundant.

## Findings

- **Source:** Code simplicity reviewer
- **Location:** `crates/mika-agent/src/rewind.rs` lines 99-201
- **Evidence:** Both functions iterate messages in reverse, collect user trace_ids into a `HashSet`, then find the earliest matching message ID. The only difference is the cross-session variant adds session-locking logic.
- **Estimated LOC reduction:** ~40 lines

## Proposed Solutions

### Option A: Delete `find_exchanges_in_messages`, use cross-session for both paths
- **Pros:** Eliminates duplication, one algorithm to maintain
- **Cons:** Slightly less obvious that the first path is single-session
- **Effort:** Small
- **Risk:** Low — cross-session function produces correct results for single-session input

## Recommended Action

Option A.

## Technical Details

- **Affected files:** `crates/mika-agent/src/rewind.rs`

## Acceptance Criteria

- [ ] `find_exchanges_in_messages` removed
- [ ] `find_recent_exchanges` calls `find_exchanges_in_messages_cross_session` for both paths
- [ ] All existing rewind tests pass
- [ ] No new test failures

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | |

## Resources

- Branch: `feat/conversation-rewind`
