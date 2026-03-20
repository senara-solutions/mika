---
status: pending
priority: p1
issue_id: 704
tags: [code-review, database]
dependencies: []
---

# a2a_get_messages historyLength Returns Oldest Instead of Most Recent

## Problem Statement

`a2a_get_messages` uses `ORDER BY id ASC LIMIT {n}` which returns the N oldest messages. The A2A protocol's `historyLength` parameter should return the N most recent messages. A client requesting `historyLength: 5` for a task with 100 messages gets messages 1-5 instead of 96-100.

## Findings

- Location: `crates/mika-agent/src/a2a_db.rs` lines 204-213
- The query orders by `id ASC` and applies `LIMIT`, returning the first N rows (oldest messages)
- The A2A protocol specifies that `historyLength` should return the most recent messages
- This means clients always see the beginning of a conversation, never the latest messages

## Proposed Solutions

Use a subquery pattern to get the N most recent messages in chronological order:

```sql
SELECT * FROM (SELECT ... ORDER BY id DESC LIMIT ?2) ORDER BY id ASC
```

This selects the N newest messages (via DESC + LIMIT) then re-orders them chronologically (via outer ASC).

## Acceptance Criteria

- [ ] `a2a_get_messages` with a limit returns the N most recent messages
- [ ] Returned messages are in ascending (chronological) order
