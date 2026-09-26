---
date: 2026-03-22
issue: 233
type: fix
---

# Fix: Work item per-session cap counts terminal items

## Problem

`count_session_work_items` counts ALL manual work items in a session regardless of status. After a sprint creates 5 work items (all completed), the agent can't create new ones.

## Fix

Add status filter to the SQL query in `db.rs:count_session_work_items()`:

```sql
AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
```

Update the test `test_create_work_item_session_cap` to verify that completing items frees up slots.

## Files

- `crates/mika-agent/src/db.rs` — `count_session_work_items()` query
- `crates/mika-agent/src/tools/create_work_item.rs` — update test
