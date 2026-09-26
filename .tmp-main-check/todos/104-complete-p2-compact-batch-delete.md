---
status: complete
priority: p2
issue_id: "104"
tags: [code-review, performance, data-integrity]
dependencies: ["099"]
---

# Use batch DELETE instead of per-event DELETE in compaction

## Problem Statement
`compact_old_memory_events` deletes old events one-by-one or by date range after processing. A single `DELETE ... WHERE created_at < ?` (or `WHERE id IN (...)`) would be O(1) instead of O(n) individual statements.

## Findings
- File: `crates/mika-agent/src/db.rs` (compact_old_memory_events function)
- Current approach issues per-event or per-group DELETE statements
- Single batch DELETE by cutoff date or collected IDs would be more efficient
- Related to #099 (transaction wrapping) — should be addressed together
- Flagged by: Data Integrity Guardian (Medium), Architecture Strategist

## Proposed Solutions

### Option 1: Single DELETE by cutoff date (Recommended)
```rust
// After all summaries inserted:
self.conn.execute(
    "DELETE FROM memory_events WHERE created_at < ?1",
    params![cutoff_date],
)?;
```
**Effort:** Small
**Risk:** Low — simpler and faster than per-row deletion

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria
- [ ] Single DELETE statement replaces per-event deletion
- [ ] Wrapped in same transaction as SELECT+INSERT (#099)
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Data Integrity Guardian and Architecture Strategist identified O(n) DELETE inefficiency
