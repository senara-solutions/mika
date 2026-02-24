---
status: pending
priority: p2
issue_id: "094"
tags: [code-review, performance, agent-native]
dependencies: []
---

# Push LIKE filter into SQL for search_memory and add reminder category

## Problem Statement
Two issues with `search_memory`:
1. When `category == "all"`, it issues 7 separate SQL queries, loads every row into memory, and performs O(N*M) substring matching in Rust. This should be pushed into SQL `LIKE` filters.
2. Reminders are not included as a searchable category. A user asking "search for anything about dentist" would miss a reminder "Call the dentist at 3pm".

## Findings
- File: `crates/mika-agent/src/tools/search_memory.rs:56-138`
- 7 separate queries when category is "all" (core_memory, people, 3x commitments by status, preferences, events)
- 3 separate commitment queries could be a single query
- No LIMIT clauses on `list_*` queries (unbounded result sets)
- Reminders are a new data category but not searchable
- Flagged by: Performance Oracle (CRITICAL-3), Agent-Native Reviewer (Warning)

## Proposed Solutions

### Option 1: Add SQL LIKE methods + reminder category (Recommended)
Add `search_*` methods to Database that accept a query parameter:
```rust
pub fn search_commitments(&self, query: &str) -> Result<Vec<Commitment>> {
    // SELECT ... FROM commitments WHERE description LIKE '%' || ?1 || '%'
}
pub fn search_reminders(&self, query: &str) -> Result<Vec<Reminder>> {
    // SELECT ... FROM reminders WHERE status = 'pending' AND message LIKE '%' || ?1 || '%'
}
```
Update `search_memory.rs` to use these instead of loading all + filtering.
**Pros:** Leverages SQLite indexing, reduces memory usage, adds reminder search
**Effort:** Medium
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/tools/search_memory.rs`

## Acceptance Criteria
- [ ] search_memory pushes LIKE filter into SQL
- [ ] 3 commitment queries consolidated into 1
- [ ] Reminder category added to search_memory
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified O(N*M) in-memory search and missing reminder category
