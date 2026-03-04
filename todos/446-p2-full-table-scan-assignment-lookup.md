---
status: complete
priority: p2
issue_id: "446"
tags: [code-review, performance]
dependencies: []
---

# Full Table Scan for Assignment Message ID Lookup

## Problem Statement

In `crates/mika-agent/src/teams/engine.rs:441-452`, `load_team_messages` fetches ALL messages for the run, then filters in Rust for assignment messages at the current iteration.

## Fix

Add `load_assignment_msg_ids(run_id, iteration) -> HashMap<String, i64>` to `db.rs` and `async_db.rs` with `WHERE message_type = 'assignment' AND iteration = ?`.

## Acceptance Criteria

- [ ] New targeted query in `db.rs`
- [ ] Async wrapper in `async_db.rs`
- [ ] `engine.rs` uses the new method
- [ ] Tests for the new query
