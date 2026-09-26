---
status: complete
priority: p3
issue_id: 303
tags: [code-review, performance, tui]
dependencies: []
---

# Add LIMIT to load_messages_after query

## Problem Statement

The `load_messages_after` DB query used by TUI cross-channel polling has no `LIMIT` clause. If many messages are inserted between poll cycles (e.g., a script rapidly calling `mika ask` in a loop), all would be loaded into memory at once. This is a minor local DoS vector against the TUI's memory usage.

## Findings

- **Security Sentinel:** The query at `db.rs` selects all rows matching `id > ?1 AND channel_type IN (...)` with no bound. Under normal usage this returns 0-2 messages, but it's unbounded in the worst case.
- **Performance Oracle:** At 0.2 queries/second with watermark-based filtering, this is not a practical concern under normal usage. The risk is only from adversarial local input (rapid `mika ask` invocations).

## Proposed Solutions

### Solution A: Add LIMIT to the query (Recommended)

Add `LIMIT 100` (or similar) to the `load_messages_after` SQL query. Messages beyond the limit are picked up in subsequent polls via the watermark.

```sql
SELECT id, role, content, channel_type, created_at
FROM conversations
WHERE id > ?1 AND role != 'summary' AND channel_type IN (...)
ORDER BY id ASC
LIMIT 100
```

- Effort: Small
- Risk: Low — surplus messages are simply deferred to the next poll cycle

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs` (load_messages_after function)

## Acceptance Criteria

- [ ] `load_messages_after` query includes a LIMIT clause
- [ ] Messages beyond the limit appear in subsequent poll cycles

## Work Log

- 2026-02-26: Created during code review of cross-channel polling + bundled skill update PR
