---
status: complete
priority: p2
issue_id: "101"
tags: [code-review, performance, data-integrity]
dependencies: []
---

# Make VACUUM conditional on actual deletions

## Problem Statement
`VACUUM` is called unconditionally on every startup in `ReminderScheduler::recover()`. SQLite VACUUM rewrites the entire database file and holds an exclusive lock for the duration. On a 100MB database this could take seconds. It should only run when compaction actually deleted rows.

## Findings
- File: `crates/mika-agent/src/scheduler.rs` (recover function)
- VACUUM runs every startup regardless of whether compaction deleted anything
- `compact_old_memory_events` returns `Result<()>` — doesn't report if rows were deleted
- SQLite VACUUM: rewrites entire DB, exclusive lock, doubles disk usage temporarily
- Flagged by: Performance Oracle, Code Simplicity Reviewer, Data Integrity Guardian

## Proposed Solutions

### Option 1: Return deletion count from compact, conditionally VACUUM (Recommended)
```rust
// In db.rs:
pub fn compact_old_memory_events(&self, retention_days: i64) -> Result<usize> {
    // ... return number of deleted rows
}

// In scheduler.rs:
let deleted = db.compact_old_memory_events(90)?;
if deleted > 0 {
    db.vacuum()?;
}
```
**Effort:** Small
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/scheduler.rs`

## Acceptance Criteria
- [ ] compact_old_memory_events returns count of deleted rows
- [ ] VACUUM only runs when rows were actually deleted
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Three agents independently flagged unconditional VACUUM as wasteful
