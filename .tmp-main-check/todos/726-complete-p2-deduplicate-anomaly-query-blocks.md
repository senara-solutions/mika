---
status: pending
priority: p2
issue_id: 726
tags: [code-review, quality, architecture]
---

# Extract helper to deduplicate 5 anomaly query blocks in get_task_health_summary

## Problem Statement

The 5 anomaly detection queries in `get_task_health_summary()` (db.rs, ~210 lines) share identical row-mapping closures, iteration loops, and push patterns. Only the SQL WHERE clause, timestamp column, anomaly_type string, and age_description format differ between them. This repetition makes the code harder to maintain and review.

## Findings

- Each query block: prepare statement → query_map with 6-tuple → iterate → format_age → push TaskHealthAnomaly
- The 6-tuple `(String, String, String, String, String, Option<String>)` mapping is copied verbatim 5 times
- The `remaining` capacity check is repeated 4 times
- Query 5 (github_linked) discards the timestamp, using a hardcoded description

## Proposed Solutions

### Option A: Extract a `query_anomalies` helper function
Helper takes SQL, params, anomaly_type, age formatter closure, limit, and output Vec.
- **Effort:** Small
- **Impact:** ~130 LOC reduction (210 → ~80)
- **Cons:** Closure for age formatting adds slight indirection

### Option B: UNION ALL single query
Combine all 5 queries into one SQL statement with anomaly_type as a computed column.
- **Effort:** Medium
- **Impact:** Eliminates 4 round-trips, removes `remaining` bookkeeping entirely
- **Cons:** More complex SQL, harder to debug individual anomaly types

**Recommended:** Option A — simpler to implement, preserves per-query debuggability.

## Acceptance Criteria

- [ ] A helper function handles the shared row-mapping and push pattern
- [ ] Each anomaly query is expressed as a concise call site
- [ ] All existing tests still pass
