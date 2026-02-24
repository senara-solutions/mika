---
status: ready
priority: p2
issue_id: "113"
tags: [code-review, performance, database]
dependencies: []
---

# Compaction Loads All Old Events Into Memory Before Grouping

## Problem Statement

`compact_old_memory_events` loads every `memory_events` row older than the cutoff into a `Vec<RawEvent>`, then iterates in Rust to build `HashMap<(year, month), MonthBucket>`. This is O(N) memory where N = number of old events. The aggregation could be done entirely in SQL using `GROUP BY`.

## Findings

- **Source:** performance-oracle (CRITICAL-3)
- **Location:** `crates/mika-agent/src/db.rs` lines 1161-1227
- **Evidence:** All events collected into `Vec<RawEvent>`, then iterated to build HashMap buckets
- **Current impact:** ~0.5-1 MB for ~4,500 rows at 90-day mark. Trivial.
- **Future impact:** If compaction fails to run, could balloon. At 100K events, ~20 MB heap allocation.

## Proposed Solutions

### Option 1: SQL GROUP BY aggregation (Recommended)
- **Pros**: O(buckets) memory instead of O(events), SQLite handles the heavy lifting
- **Cons**: Slightly more complex SQL, category extraction needs SQL string functions
- **Effort**: Medium
- **Risk**: Low

```sql
-- Tool counts per month
SELECT strftime('%Y', created_at) AS year,
       strftime('%m', created_at) AS month,
       tool_name, COUNT(*) as cnt
FROM memory_events WHERE created_at < ?1
GROUP BY year, month, tool_name;

-- Category counts per month
SELECT strftime('%Y', created_at) AS year,
       strftime('%m', created_at) AS month,
       substr(target_key, 1, CASE WHEN instr(target_key, ':') > 0
              THEN instr(target_key, ':') - 1
              ELSE length(target_key) END) AS category,
       COUNT(*) as cnt
FROM memory_events WHERE created_at < ?1
GROUP BY year, month, category;
```

### Option 2: Keep Rust-side grouping with streaming iterator
- **Pros**: No SQL change, simpler mental model
- **Cons**: Still O(N) iteration, just avoids Vec allocation
- **Effort**: Small
- **Risk**: Low

## Recommended Action

_To be filled during triage_

## Technical Details

- **Affected Files**: `crates/mika-agent/src/db.rs` (compact_old_memory_events)
- **Database Changes**: No schema changes, only query changes

## Acceptance Criteria

- [ ] Compaction no longer collects all events into a Vec
- [ ] Monthly summaries produce identical results (tool_counts, category_counts)
- [ ] All 6 compaction tests pass
- [ ] Memory usage is O(months) not O(events)

## Work Log

### 2026-02-24 - Identified in v4 Code Review
**By:** Multi-agent review (performance-oracle)

## Resources

- Commit under review: 38a843b
