---
status: pending
priority: p3
issue_id: "054"
tags: [code-review, quality, database, rust-v2]
dependencies: []
---

# filter_map Pattern Should Be Simplified to collect

## Problem Statement
With encryption removed, the filter_map warn-and-skip pattern in db.rs is no longer justified. 5 methods still use `filter_map` with `match Ok/Err` to handle row deserialization, but plaintext rows should never fail to parse. Replace with `.collect::<rusqlite::Result<Vec<_>>>()?` which propagates errors directly (~30 lines saved).

**Location:** `crates/mika-agent/src/db.rs` — `load_recent_messages`, `get_all_core_memory`, `list_people`, `list_commitments`, `get_memory_events`

**Reported by:** code-simplicity-reviewer, architecture-strategist

## Proposed Solutions

### Option A: Replace filter_map with collect (Recommended)
Replace each `filter_map(|row| match row { Ok(r) => ... Err(e) => { warn!(...); None } })` with `.collect::<rusqlite::Result<Vec<_>>>()?`.
- **Pros:** Simpler, propagates real errors, ~30 lines saved
- **Cons:** None — plaintext rows should never fail
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria
- [ ] All 5 methods use `.collect::<rusqlite::Result<Vec<_>>>()?`
- [ ] No filter_map warn-and-skip patterns remain in db.rs
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | ~150 lines of structural duplication |
| 2026-02-24 | Context updated — encryption removed, pattern now unjustified. Downgraded from decrypt pattern to simple collect | code-simplicity-reviewer, architecture-strategist |
