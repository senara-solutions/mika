---
status: pending
priority: p2
issue_id: "655"
tags: [code-review, quality]
dependencies: []
---

# `run_gh`: Missing 10,000-character input length validation

## Problem Statement

The project convention is "Each tool validates inputs (empty check + 10,000 char max)". The `web_search` builtin enforces this on the `query` parameter. The `run_gh` handler validates emptiness but does not enforce the 10,000-character limit on individual command elements or total command length.

## Findings

- **Pattern recognition**: Flagged as a deviation from project convention.
- **Architecture reviewer**: Confirmed the gap.
- **Agent-native reviewer**: Noted the missing validation.

## Proposed Solutions

### Solution 1: Total length check on command array (Recommended)
Add a check on the total serialized length of all command elements:
```rust
let total_len: usize = command.iter().map(|s| s.len()).sum();
if total_len > 10_000 {
    return ToolOutput::error("Command too long (max 10000 characters total).".to_string());
}
```
- **Pros**: Consistent with project convention, simple
- **Cons**: None
- **Effort**: Small
- **Risk**: Low

## Recommended Action

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/builtin_handlers.rs`
- **Components**: `run_gh` input validation

## Acceptance Criteria

- [ ] Total command length validated against 10,000 character limit
- [ ] Test covers the length validation

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Flagged by 3 reviewers |
