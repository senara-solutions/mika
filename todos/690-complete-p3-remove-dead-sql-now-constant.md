---
status: pending
priority: p3
issue_id: "690"
tags: [code-review, cleanup]
dependencies: []
---

## Problem Statement

`timestamp::SQL_NOW` constant is defined but never used anywhere in the codebase. All SQL statements inline the `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')` expression directly. This is dead code.

## Findings

Found by: code-simplicity-reviewer, pattern-recognition-specialist

- `crates/mika-agent/src/timestamp.rs` line 8: `pub const SQL_NOW: &str = ...`
- Zero references outside the module

## Proposed Solutions

### Option A: Remove the constant

- **Effort:** Trivial
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-agent/src/timestamp.rs`

## Acceptance Criteria

- [ ] `SQL_NOW` constant removed
- [ ] Code compiles

## Work Log

- 2026-03-18: Identified during code review
