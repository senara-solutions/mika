---
status: pending
priority: p2
issue_id: "586"
tags: [code-review, performance]
dependencies: []
---

# TimelineFilters::to_sql() Passes Integer Timestamps as Strings

## Problem Statement
The `from` and `to` filters are `Option<i64>` but converted to `String` via `to_string()` and stored in `Vec<String>`. SQLite must perform string-to-integer comparison against the `created_at` integer column, preventing index usage.

## Findings
- **Source:** Performance Oracle
- **Location:** `crates/mika-agent/src/db.rs` lines 310-316

## Proposed Solutions
Use `Vec<Box<dyn ToSql>>` or `Vec<rusqlite::types::Value>` to pass integers natively.

## Acceptance Criteria
- [ ] Integer timestamps passed as integers to SQLite
- [ ] Indexes on created_at can be used for time range filters

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Performance Oracle found type mismatch |

## Resources
- PR #89
