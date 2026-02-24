---
status: ready
priority: p2
issue_id: "065"
tags: [code-review, agent-native, quality, rust-v2]
dependencies: []
---

# Event Search Results Missing ID in Output

## Problem Statement

`search_memory` formats person results as `[person] Name (id:1)` and commitment results as `[commitment] Desc (id:1, status:pending)`, but event results are formatted as `[event] Description (date)` without the `id`. This inconsistency prevents the agent from targeting specific events for future updates via `update_fact`.

**Why it matters:** Agent-native parity requires IDs in search output for all entity types that support update/delete operations.

## Findings

- **Source:** agent-native-reviewer, pattern-recognition-specialist
- **Location:** `crates/mika-agent/src/tools/search_memory.rs:131`
- **Evidence:** `format!("[event] {}", event.description)` — no `id` field included, while persons (line 78) and commitments (line 97) include IDs

## Proposed Solutions

### Option A: Add id to event format string (Recommended)
- Change to `format!("[event] {} (id:{})", event.description, event.id)`
- Include date after id: `[event] Board meeting (id:3, 2026-03-15)`
- **Pros:** Consistent with person/commitment format, enables future event updates
- **Cons:** ~2 lines changed
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Event search results include `(id:X)` in output
- [ ] Format is consistent with person and commitment patterns
- [ ] Existing search tests updated
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | All searchable entities with IDs should expose them in output |
