---
status: complete
priority: p2
issue_id: "088"
tags: [code-review, security, performance]
dependencies: []
---

# Add tiered retention for memory_events table

## Problem Statement
The `memory_events` audit table grows without bound. Every tool call that mutates memory appends a row. Over the lifetime of a customer container, this table can grow to millions of rows, causing disk space exhaustion and performance degradation.

However, Mika is an assistant-for-life product. The evolution history has future value for agent self-reflection and improvement. Simple pruning would destroy this value. Instead, use tiered retention: compress old events into monthly summaries, keep summaries forever.

## Findings
- File: `crates/mika-agent/src/db.rs:991-1008` (log_memory_event)
- `heartbeat_sends` has `prune_old_heartbeat_sends(days)` but `memory_events` has no equivalent
- `memory_events` is currently write-only in production (no reader besides tests)
- In per-customer container architecture, unbounded growth is a slow-burn DoS vector
- Flagged by: Security Sentinel (Medium)
- Brainstorm: `docs/brainstorms/2026-02-24-memory-events-tiered-retention-brainstorm.md`

## Proposed Solutions

### Option 1: Tiered retention — compress, don't delete (Recommended)

**Tier 1 — Raw events (0-90 days):** Keep full-detail rows for debugging and near-term agent use.

**Tier 2 — Monthly summaries (forever):** After 90 days, compact into `memory_event_summaries` table:

```sql
CREATE TABLE memory_event_summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    year INTEGER NOT NULL,
    month INTEGER NOT NULL,
    tool_counts TEXT NOT NULL,        -- JSON: {"store_fact": 42, "update_core_memory": 7, ...}
    category_counts TEXT NOT NULL,    -- JSON: {"person": 15, "commitment": 20, ...}
    total_mutations INTEGER NOT NULL,
    top_targets TEXT NOT NULL,        -- JSON: ["person:Alice Chen", "commitment:Q4 budget", ...]
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(year, month)
);
```

New methods in db.rs:
```rust
pub fn compact_old_memory_events(&self, days: u32) -> Result<()> {
    // 1. Query events older than `days`, grouped by year/month
    // 2. For each month: aggregate tool_counts, category_counts, total, top_targets
    // 3. INSERT OR REPLACE into memory_event_summaries
    // 4. DELETE compacted raw events
    // 5. All within a transaction
}

pub fn db_size_bytes(&self) -> Result<u64> {
    // Return database file size for health monitoring
}

pub fn get_memory_event_summaries(&self, year: Option<i32>) -> Result<Vec<MemoryEventSummary>> {
    // Read summaries (for future agent use)
}
```

Wire into scheduler.rs::recover():
```rust
if let Err(e) = self.db.compact_old_memory_events(90) {
    warn!(error = %e, "failed to compact old memory events");
}
if let Err(e) = self.db.vacuum() {
    warn!(error = %e, "failed to vacuum database");
}
let size = self.db.db_size_bytes()?;
if size > 500_000_000 {
    warn!(size_bytes = size, "database size exceeds 500MB");
}
```

**Pros:** Preserves evolution history forever, bounds raw storage to 90 days, enables future agent self-reflection
**Cons:** More complex than simple pruning
**Effort:** Medium
**Risk:** Low

### ~~Option 2: Simple pruning~~ (Rejected)
~~Delete events older than 30 days.~~
**Rejected:** Destroys evolution history that has future value for an assistant-for-life product.

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs` (new table, 3 new methods), `crates/mika-agent/src/scheduler.rs` (wire compaction + health check)

## Acceptance Criteria
- [ ] `memory_event_summaries` table added to schema
- [ ] `compact_old_memory_events(days)` aggregates and deletes within a transaction
- [ ] `db_size_bytes()` returns file size, warns at 500MB
- [ ] `get_memory_event_summaries()` reads summaries
- [ ] Called during scheduler recovery with 90-day threshold
- [ ] VACUUM runs after compaction
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified unbounded audit table growth with no pruning mechanism

### 2026-02-24 - Brainstorm
**By:** User + Claude Code
**Actions:** Rejected simple pruning in favor of tiered retention. Mika is an assistant-for-life product — compress, don't delete. See brainstorm doc.
