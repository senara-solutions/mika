---
status: complete
priority: p2
issue_id: 227
tags: [code-review, performance, slash-commands]
dependencies: []
---

# Sequential DB Queries Should Use tokio::join!

## Problem Statement

`handle_memory_search` makes 4 sequential DB queries (people, commitments, preferences, events) and `handle_status` makes 4 sequential queries (count, size, tokens, schema). These could run concurrently via `tokio::join!` for better latency.

**Why it matters:** Each DB query involves a channel round-trip to the DB thread. Running them in parallel could reduce latency by ~3-4x for these commands.

## Findings

**Source:** Performance Oracle review agent

**Locations:**
- `crates/mika-cli/src/tui/commands/handlers.rs:109-159` (`handle_memory_search`) — 4 sequential `.await` calls
- `crates/mika-cli/src/tui/commands/handlers.rs:192-216` (`handle_status`) — 4 sequential `.await` calls

## Proposed Solutions

### Solution A: Use tokio::join! for concurrent queries (Recommended)
- Wrap all 4 queries in `tokio::join!()` for each handler
- **Pros:** Simple, idiomatic, significant latency improvement
- **Cons:** Slightly more complex error handling
- **Effort:** Small
- **Risk:** Low (DB queries are independent, AsyncDatabase is Clone)

```rust
let (people, commitments, preferences, events) = tokio::join!(
    app.db.search_people(query),
    app.db.search_commitments(query),
    app.db.search_preferences(query),
    app.db.search_events(query),
);
```

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/commands/handlers.rs`

## Acceptance Criteria

- [ ] handle_memory_search uses tokio::join! for concurrent queries
- [ ] handle_status uses tokio::join! for concurrent queries
- [ ] Output remains identical

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from code review | Performance oracle flagged sequential queries |

## Resources

- PR branch: `feat/slash-commands`
