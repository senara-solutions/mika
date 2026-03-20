---
status: pending
priority: p1
issue_id: 701
tags: [code-review, security, database]
dependencies: []
---

# SQL Format String Used for LIMIT Clause

## Problem Statement

In `a2a_db.rs` line 208, the LIMIT clause uses `format!("... LIMIT {n}")` instead of a parameterized query. While `n` is `i32` (preventing string injection), this violates the codebase convention of using parameterized queries everywhere. A negative `i32` value is also unhandled (SQLite treats negative LIMIT as "no limit").

## Findings

- Location: `crates/mika-agent/src/a2a_db.rs` lines 204-213
- The LIMIT value is interpolated via `format!()` rather than bound as a parameter
- All other queries in the codebase use `rusqlite::params!` for value binding
- Negative `i32` values pass through unchecked; SQLite interprets negative LIMIT as unlimited rows

## Proposed Solutions

Use a parameterized query with `rusqlite::params!` for the LIMIT value. Clamp negative values to 0.

## Acceptance Criteria

- [ ] LIMIT clause uses a parameterized query via `rusqlite::params!`
- [ ] Negative values are clamped to 0 before being passed to the query
