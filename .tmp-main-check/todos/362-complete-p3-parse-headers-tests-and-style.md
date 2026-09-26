---
status: complete
priority: p3
issue_id: "362"
tags: [code-review, testing, mcp]
dependencies: []
---

# Add tests for parse_headers and fix HashMap import style

## Problem Statement

The `parse_headers` function handles several edge cases but has no unit tests. Also, `std::collections::HashMap` is used via full path instead of a `use` import, inconsistent with the rest of the codebase.

## Findings

- **Source**: pattern-recognition-specialist review
- **File**: `crates/mika-cli/src/commands/mcp.rs`
- **Evidence**: Lines 144-163 have no test coverage; lines 146, 152 use full path `std::collections::HashMap`

## Proposed Solutions

### Option A: Add tests and fix import (Recommended)

1. Add `use std::collections::HashMap;` import
2. Add unit tests for: normal KEY=VALUE, value with `=`, missing `=`, empty key, empty vector

- Effort: Small
- Risk: None

## Acceptance Criteria

- [ ] `parse_headers` has unit tests covering edge cases
- [ ] `HashMap` imported via `use` instead of full path

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-01 | Created from code review | |
