---
status: pending
priority: p2
issue_id: "730"
tags: [code-review, quality, observability]
dependencies: []
---

# Rename `TOOL_CALL_MAX_CHARS` to `TOOL_CALL_MAX_BYTES`

## Problem Statement

The constant is named `TOOL_CALL_MAX_CHARS` but the truncation logic operates on byte boundaries (`s.len()` returns bytes, `is_char_boundary` is byte-level). The naming is misleading.

## Findings

- **Agents**: performance-oracle, architecture-strategist, code-simplicity-reviewer
- **File**: `crates/mika-agent/src/db.rs` line 3390

## Proposed Solutions

Rename constant to `TOOL_CALL_MAX_BYTES`. One-line change.

## Acceptance Criteria

- [ ] Constant renamed
- [ ] Compiles and tests pass
