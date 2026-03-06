---
status: pending
priority: p3
issue_id: "476"
tags: [code-review, quality, task-engine]
dependencies: []
---

# 476 · Task status and action type are magic strings throughout — no constants or enum

## Problem Statement

Task status values (`"pending"`, `"in_progress"`, `"completed"`, `"failed"`,
`"cancelled"`, `"expired"`, `"recurring_active"`) and action types
(`"send_message"`, `"run_skill"`, `"inject_context"`, `"resume_agent"`,
`"invoke_orchestrator"`) are repeated as bare string literals across
`engine.rs`, `dispatcher.rs`, `db.rs`, and `async_db.rs`. A typo compiles
silently. The DB schema validates them on INSERT, but runtime mismatches
produce confusing `no rows updated` behavior rather than a type error.

## Findings

- **Locations:** `task_engine/engine.rs`, `task_engine/dispatcher.rs`, `db.rs`, `async_db.rs` — dozens of string literals

## Proposed Solutions

### Option A — Constants module in `task_engine/types.rs`
```rust
pub mod task_status {
    pub const PENDING: &str = "pending";
    pub const IN_PROGRESS: &str = "in_progress";
    // ...
}
pub mod action_type {
    pub const SEND_MESSAGE: &str = "send_message";
    // ...
}
```

**Effort:** Small | **Risk:** Low

### Option B — Enum with `as_str()` method
Full type safety; serde integration for DB round-trip.
**Effort:** Medium | **Risk:** Low (but broader refactor)

## Recommended Action

Option A initially. Enums are a good follow-up.

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/` (new `types.rs`), `db.rs`

## Acceptance Criteria

- [ ] Constants defined for all status values and action types
- [ ] All string literals in task_engine/ replaced with constants
- [ ] `cargo clippy` passes with no new warnings

## Work Log

- 2026-03-06: Identified by code quality review agent (QUAL-7)
