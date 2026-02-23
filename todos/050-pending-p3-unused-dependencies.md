---
status: pending
priority: p3
issue_id: "050"
tags: [code-review, dependencies, rust-v2]
dependencies: []
---

# Unused and Misplaced Crate Dependencies

## Problem Statement
- `chrono` is declared as workspace dependency but not imported anywhere (all dates use SQLite's `datetime('now')`)
- `uuid` is declared in `mika-common/Cargo.toml` but only used in `mika-agent/src/cli.rs`
- `tool_calls()` method on `MessagesResponse` (claude.rs:129-136) is tested but never called in production code

**Reported by:** code-simplicity-reviewer

## Proposed Solutions
- Remove `chrono` from workspace dependencies
- Move `uuid` from mika-common to mika-agent only
- Remove `tool_calls()` method and its test (or keep if Phase 2 will use it)
- **Effort:** Small

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
