---
status: complete
priority: p2
issue_id: "416"
tags: [code-review, performance, database, reflection]
dependencies: ["413"]
---

# No LIMIT Clause on get_conversations_since SQL Query

## Problem Statement

`get_conversations_since()` loads ALL messages since midnight into a `Vec<ConversationMessage>` in memory with no SQL LIMIT. The 50K char cap is only enforced in the string builder in `agent.rs`. For high-volume agents, thousands of messages (with large content fields) are loaded and immediately discarded.

## Findings

- **Performance oracle**: "500 messages/day with average 2KB content = ~1MB of String allocations loaded into memory, only to be truncated to 50K chars"
- **Security sentinel**: "Adding a LIMIT to the SQL query would provide defense in depth against degenerate cases"

## Proposed Solutions

### Option A: Add LIMIT clause (Recommended)
```sql
SELECT ... FROM conversations WHERE created_at >= ?1 AND channel_type != 'reflection' ORDER BY id LIMIT 500
```
- **Effort**: Small (1 SQL clause)
- **Risk**: Low (digest builder still provides the real truncation)

## Technical Details

- **Affected file**: `crates/mika-agent/src/db.rs` (line 1098)

## Acceptance Criteria

- [ ] SQL query includes LIMIT clause
- [ ] Existing tests still pass
