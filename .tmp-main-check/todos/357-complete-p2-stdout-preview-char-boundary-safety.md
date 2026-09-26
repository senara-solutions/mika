---
status: complete
priority: p2
issue_id: "357"
tags: [code-review, safety, executor]
dependencies: []
---

# Fix stdout_preview Char-Boundary Panic Risk

## Problem Statement

In `executor.rs`, the debug logging slices stdout/stderr with `&stdout[..stdout.len().min(200)]` which could panic if the 200-byte boundary falls in the middle of a multi-byte UTF-8 character. While `String::from_utf8_lossy` produces valid UTF-8, the replacement character U+FFFD is 3 bytes, so a boundary at byte 200 could land mid-character.

## Findings

- **Pattern Recognition Specialist**: The existing `truncate_output()` function already handles char-boundary slicing correctly with a `while !s.is_char_boundary(boundary)` loop. The same pattern should be applied to the debug logging preview.
- **Source**: `crates/mika-agent/src/skills/executor.rs` lines 272, 278

## Proposed Solutions

### Solution A: Use char-boundary-safe slicing (Recommended)
Extract a small helper or inline the boundary check from `truncate_output()`.

- **Pros**: Eliminates theoretical panic, follows existing codebase pattern
- **Cons**: A few extra lines
- **Effort**: Small
- **Risk**: None

## Recommended Action

Solution A — inline boundary check.

## Acceptance Criteria

- [x] `stdout_preview` slicing handles multi-byte char boundaries safely
- [x] `stderr` slicing handles multi-byte char boundaries safely

## Work Log

- 2026-02-28: Found during code review, fixed immediately
