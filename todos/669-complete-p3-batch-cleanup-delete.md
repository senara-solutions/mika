---
status: pending
priority: p3
issue_id: 669
tags: [code-review, performance, gateway]
dependencies: []
---

# Batch outbound_messages Cleanup DELETE with LIMIT

## Problem Statement

The cleanup query `DELETE FROM outbound_messages WHERE created_at < now() - interval '7 days'` runs without a LIMIT. At high message volumes (50K+/day), this could delete hundreds of thousands of rows in a single transaction, causing WAL traffic and competing with concurrent INSERTs.

## Findings

- `crates/mika-gateway/src/routes.rs:738` — unbounded DELETE

Identified by: performance-oracle

## Proposed Solutions

Add a LIMIT to batch the cleanup:

```sql
DELETE FROM outbound_messages
WHERE ctid IN (
    SELECT ctid FROM outbound_messages
    WHERE created_at < now() - interval '7 days'
    LIMIT 1000
)
```

- **Effort**: Small
- **Risk**: None — current volumes are well below the threshold
