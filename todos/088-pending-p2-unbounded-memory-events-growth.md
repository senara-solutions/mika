---
status: pending
priority: p2
issue_id: "088"
tags: [code-review, security, performance]
dependencies: []
---

# Add memory_events table pruning

## Problem Statement
The `memory_events` audit table grows without bound. Every tool call that mutates memory appends a row. Over the lifetime of a customer container, this table can grow to millions of rows, causing disk space exhaustion and performance degradation. The previously filed todo #057 was deleted with no replacement solution.

## Findings
- File: `crates/mika-agent/src/db.rs:991-1008` (log_memory_event)
- `heartbeat_sends` has `prune_old_heartbeat_sends(days)` but `memory_events` has no equivalent
- In per-customer container architecture, this is a slow-burn DoS vector
- Flagged by: Security Sentinel (Medium)

## Proposed Solutions

### Option 1: Add prune method + wire into scheduler (Recommended)
```rust
pub fn prune_old_memory_events(&self, days: u32) -> Result<()> {
    let modifier = format!("-{days} days");
    self.conn.execute(
        "DELETE FROM memory_events WHERE created_at < datetime('now', ?1)",
        [&modifier],
    )?;
    Ok(())
}
```
Call from `scheduler.rs::recover()` next to `prune_old_heartbeat_sends(7)`.
**Pros:** Consistent with existing heartbeat pruning pattern
**Cons:** None
**Effort:** Small
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/scheduler.rs`

## Acceptance Criteria
- [ ] `prune_old_memory_events(days)` method added to Database
- [ ] Called during scheduler recovery (suggest 30-day retention)
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified unbounded audit table growth with no pruning mechanism
