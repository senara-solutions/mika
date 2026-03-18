---
status: pending
priority: p3
issue_id: "692"
tags: [code-review, cleanup]
dependencies: []
---

## Problem Statement

In `timestamp.rs`, `use chrono::Datelike;` is placed after the closing brace of `test_now_plus_minus` (line 77), outside any test function but inside the test module. It should be at the top of `mod tests` or inside the specific test that uses it (`test_parse_iso8601`).

## Findings

Found by: code-simplicity-reviewer

## Proposed Solutions

Move `use chrono::Datelike;` to the top of the `mod tests` block.

- **Effort:** Trivial
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-agent/src/timestamp.rs`

## Acceptance Criteria

- [ ] Import moved to proper location
- [ ] Code compiles

## Work Log

- 2026-03-18: Identified during code review
