---
status: complete
priority: p3
issue_id: 429
tags: [code-review, quality, dead-code]
dependencies: []
---

# AsyncDatabase::in_memory() is dead code

## Problem Statement

`AsyncDatabase::in_memory()` was added for team mode TUI but the implementation switched to on-disk DB (`AsyncDatabase::open()`). No production or test code calls `in_memory()` — tests use `Database::open_in_memory()` + `AsyncDatabase::new(db)` directly via `test_utils::test_async_db()`.

## Findings

- Source: code-simplicity-reviewer
- Location: `crates/mika-agent/src/async_db.rs` lines 70-74
- Doc comment already updated to say "primarily for tests; team mode now uses on-disk DB"
- 6 lines of dead code

## Proposed Solutions

### Option A: Remove the method (Recommended)
- Delete `AsyncDatabase::in_memory()`
- **Pros:** Less confusion, cleaner API
- **Effort:** Small (6 lines)

### Option B: Keep as convenience for tests
- Already handled by test_utils, so adds no value
- **Pros:** None meaningful
- **Effort:** N/A

## Acceptance Criteria

- [ ] No unused public methods on AsyncDatabase
- [ ] All tests still pass
