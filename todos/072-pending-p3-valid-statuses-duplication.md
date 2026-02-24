---
status: pending
priority: p3
issue_id: "072"
tags: [code-review, quality, rust-v2]
dependencies: []
---

# VALID_STATUSES Defined in Two Places with Different Values

## Problem Statement

`VALID_STATUSES` is defined in both `update_fact.rs` (`["completed", "cancelled"]`) and `db.rs` (`["pending", "completed", "cancelled"]`). The DB allows "pending" for internal operations while the tool restricts to terminal states. The relationship between the two sets is implicit — adding a status like "deferred" requires updating both.

## Findings

- **Source:** pattern-recognition-specialist
- **Location:** `crates/mika-agent/src/tools/update_fact.rs:8`, `crates/mika-agent/src/db.rs:461`

## Proposed Solutions

### Option A: Shared constant with tool-level filtering (Recommended)
- Define `COMMITMENT_STATUSES` in `db.rs` with all values
- Tool filters to exclude "pending" explicitly
- **Pros:** Single source of truth, relationship is explicit
- **Effort:** Small
- **Risk:** None

### Option B: Keep separate with comment
- Add comment in each location referencing the other
- **Pros:** Simplest
- **Effort:** Tiny
- **Risk:** Low (drift)

## Acceptance Criteria

- [ ] Status values defined in one place or explicitly linked
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | Related constants in different files should reference each other |
