---
status: complete
priority: p1
issue_id: "039"
tags: [code-review, bug, tools, database, rust-v2]
dependencies: []
---

# Events Are Write-Only Data Sink

## Problem Statement

The `add_event()` method in `Database` stores events, but there is no `list_events()` method and `search_memory` does not search events at all. Events can be stored via `store_fact` but never retrieved or searched. This makes the event category a write-only data sink.

**Location:**
- `crates/mika-agent/src/db.rs` - `add_event()` exists but no `list_events()`
- `crates/mika-agent/src/tools/search_memory.rs` - Event search path is missing entirely

**Reported by:** agent-native-reviewer

## Findings

- `db.add_event()` at db.rs:863 stores events with encrypted description, date, and notes
- `search_memory.rs` searches core_memory, people, commitments, and preferences but NOT events
- The events table has encrypted fields that need decrypt-then-filter like other categories
- `store_fact` with `category: "event"` works correctly for writing

## Proposed Solutions

### Option A: Add list_events() and wire into search_memory (Recommended)
Add a `list_events()` method to `Database` (mirroring `list_people` and `list_commitments`) and add event searching to `search_memory` with the same decrypt-then-filter pattern.
- **Pros:** Complete feature parity, consistent patterns
- **Cons:** None significant
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] `Database` has a `list_events()` method returning all events
- [ ] `search_memory` searches events when `category` is "all" or "event"
- [ ] Test: store an event, search for it, find it

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | Event category was added in Phase 6 but search path was missed |
