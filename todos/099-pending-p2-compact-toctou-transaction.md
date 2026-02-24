---
status: pending
priority: p2
issue_id: "099"
tags: [code-review, data-integrity, concurrency]
dependencies: []
---

# Wrap compact_old_memory_events SELECT+DELETE in transaction

## Problem Statement
`compact_old_memory_events()` performs a SELECT to find old events, then later DELETEs them by date range. In Phase 1 (single-threaded CLI), this is safe. In Phase 2 (async server with concurrent requests), new events could be inserted between SELECT and DELETE, causing data loss (TOCTOU race).

## Findings
- File: `crates/mika-agent/src/db.rs` (compact_old_memory_events function)
- SELECT finds events older than cutoff → aggregates in Rust → INSERT summaries → DELETE by date range
- Gap between SELECT and DELETE allows concurrent INSERTs to be caught by DELETE
- Phase 1 is single-threaded so this is safe today
- Phase 2 with AsyncDatabase will introduce concurrency
- Flagged by: Data Integrity Guardian (CRITICAL for Phase 2)

## Proposed Solutions

### Option 1: Wrap in transaction (Recommended)
```rust
let tx = self.conn.unchecked_transaction()?;
// SELECT, aggregate, INSERT summaries, DELETE originals all within tx
tx.commit()?;
```
**Effort:** Small
**Risk:** Low — `unchecked_transaction` already used elsewhere in codebase

### Option 2: DELETE by collected IDs instead of date range
```rust
let ids: Vec<i64> = events.iter().map(|e| e.id).collect();
// DELETE FROM memory_events WHERE id IN (...)
```
**Effort:** Small
**Risk:** Low — eliminates TOCTOU regardless of transaction

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria
- [ ] SELECT and DELETE are within same transaction
- [ ] No TOCTOU window for concurrent inserts
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Data Integrity Guardian identified TOCTOU risk for Phase 2 concurrency
