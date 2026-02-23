---
status: pending
priority: p2
issue_id: "045"
tags: [code-review, testing, quality, rust-v2]
dependencies: []
---

# Test Helper Duplication Across 4 Modules

## Problem Statement
`test_key()`, `test_db()`, and `test_ctx()` functions are duplicated across db.rs, store_fact.rs, search_memory.rs, and update_core_memory.rs (~80 lines of identical code).

**Reported by:** pattern-recognition-specialist

## Proposed Solutions

### Option A: Create a test utilities module (Recommended)
Add `#[cfg(test)] pub mod test_utils` to lib.rs with shared test helpers.
- **Pros:** Single source of truth, easier maintenance
- **Cons:** Minor refactor
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria
- [ ] Shared test helpers in one location
- [ ] All 4 modules use shared helpers
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
