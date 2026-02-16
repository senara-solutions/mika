---
status: complete
priority: p2
issue_id: "090"
tags: [code-review, architecture, quality]
dependencies: []
---

# Use rusqlite::Transaction in replace_with_summary

## Problem Statement
`replace_with_summary()` uses manual `BEGIN`/`COMMIT`/`ROLLBACK` via raw SQL instead of rusqlite's `Transaction` type. If ROLLBACK fails, the `?` operator masks the original error. If a panic occurs between BEGIN and the match, the connection is left in a transaction state.

## Findings
- File: `crates/mika-agent/src/db.rs:886-899`
- Uses closure pattern `(|| { ... })()` for transaction body
- `rusqlite::Connection::transaction()` provides RAII-based automatic rollback on drop
- SQLite auto-rolls back on connection close, so impact is low for single-user CLI
- Flagged by: Security Sentinel (Low), Architecture Strategist (Medium), Pattern Recognition (P3)

## Proposed Solutions

### Option 1: Use rusqlite::Transaction (Recommended)
```rust
pub fn replace_with_summary(&self, summary: &str, compacted_through_id: i64) -> Result<i64> {
    let tx = self.conn.unchecked_transaction()?;
    tx.execute("DELETE FROM conversations WHERE role = 'summary'", [])?;
    let rows = tx.execute("DELETE FROM conversations WHERE id <= ?1 AND role != 'summary'", [compacted_through_id])?;
    tx.execute(
        "INSERT INTO conversations (role, content, channel_type, compacted_through_id) VALUES ('summary', ?1, 'system', ?2)",
        rusqlite::params![summary, compacted_through_id],
    )?;
    let id = self.conn.last_insert_rowid();
    tx.commit()?;
    Ok(id)
}
```
Note: `unchecked_transaction()` needed because `self.conn` is `Connection` not `&mut Connection`.
**Pros:** RAII cleanup, automatic rollback on drop/panic
**Cons:** Need `unchecked_transaction()` since `Connection` uses interior mutability
**Effort:** Small
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria
- [ ] `replace_with_summary` uses `rusqlite::Transaction` or `unchecked_transaction()`
- [ ] No raw `BEGIN`/`COMMIT`/`ROLLBACK` in DML methods
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified manual transaction management that could mask errors
