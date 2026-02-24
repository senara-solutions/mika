---
title: "Memory Events: Tiered Retention Instead of Pruning"
date: 2026-02-24
status: decided
related_todos:
  - "088-pending-p2-unbounded-memory-events-growth"
---

# Memory Events: Tiered Retention Instead of Pruning

## What We're Building

A tiered retention system for the `memory_events` audit table that compresses old events into monthly summaries instead of deleting them. Mika is an assistant-for-life product — the evolution history has future value for agent self-reflection and improvement.

### Current State

- `memory_events` is a write-only audit log (5 tools, 8 call sites)
- No production code reads from it (only test assertions)
- Grows without bound — no retention policy
- Stores: session_id, tool_name, target_key, before_value, after_value, reasoning, created_at

### Design: Two-Tier Retention

**Tier 1 — Raw events (0-90 days):**
Keep full-detail `memory_events` rows for 90 days. These support debugging, forensics, and near-term agent self-reflection.

**Tier 2 — Monthly summaries (forever):**
After 90 days, compact into a `memory_event_summaries` table:
- month, year
- event counts by tool_name
- event counts by target category (person, commitment, preference, event, reminder, core_memory)
- total mutations
- key themes (most-mutated target_keys)

Summaries are kept forever. Raw events are deleted after successful compaction.

## Why This Approach

1. **Lifetime product**: Users keep Mika forever. Deleting the audit trail loses the story of how Mika's understanding evolved.
2. **Future agent value**: Monthly summaries enable future features like "I updated your priorities 12 times last month" or "Your commitment completion rate improved in Q3."
3. **Bounded storage**: Raw events are bounded to 90 days. Monthly summaries are tiny (~1 row/month).
4. **Mirrors conversation compaction**: Same philosophy as the existing conversation compaction — summarize, don't delete.

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Retention strategy | Tiered (compress, don't delete) | Assistant-for-life product |
| Raw retention period | 90 days | Sufficient for debugging, bounded growth |
| Summary granularity | Monthly | Good balance of detail vs storage |
| Summary lifetime | Forever | Tiny rows, high future value |
| Compaction trigger | scheduler.rs::recover() (startup) | Consistent with heartbeat pruning pattern |
| Post-compaction cleanup | VACUUM | Reclaim disk space from deleted rows |
| Health monitoring | db_size_bytes() with 500MB warning | Early warning for unexpected growth |

## Implementation Sketch

### New table: `memory_event_summaries`

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

### New methods in db.rs

- `compact_old_memory_events(days: u32)` — aggregate events older than N days into monthly summaries, delete raw events
- `db_size_bytes()` — return database file size for health monitoring
- `get_memory_event_summaries(year: Option<i32>)` — read summaries (for future agent use)

### Wiring in scheduler.rs::recover()

```rust
if let Err(e) = self.db.compact_old_memory_events(90) {
    warn!(error = %e, "failed to compact old memory events");
}
// VACUUM after compaction
if let Err(e) = self.db.vacuum() {
    warn!(error = %e, "failed to vacuum database");
}
let size = self.db.db_size_bytes()?;
if size > 500_000_000 {
    warn!(size_bytes = size, "database size exceeds 500MB");
}
```

## Open Questions

None — requirements are clear.

## Supersedes

This brainstorm supersedes the original proposed solution in todo #088, which suggested simple pruning with `prune_old_memory_events(days)`. The todo should be updated to reflect the tiered retention approach.
