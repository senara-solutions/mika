---
status: complete
priority: p3
issue_id: "598"
tags: [code-review, performance, tui]
dependencies: []
---

# Avoid Vec<char> allocation in screen_to_textarea_pos on every mouse drag

## Problem Statement

`screen_to_textarea_pos()` in `input.rs:59` collects each line's characters into a `Vec<char>` on every mouse drag event. Terminals can send dozens of drag events per second. With a 100KB paste (the configured limit), a single line could be up to 100K characters, creating significant allocation pressure.

## Findings

- Performance Oracle flagged this as a critical performance issue at scale
- The `Vec<char>` exists solely to enable slice indexing in `find_char_at_col`
- `find_char_at_col` only performs a forward walk — it doesn't need random access

## Proposed Solutions

### Option A: Iterate chars() directly without collecting
Rewrite `find_char_at_col` to accept `impl Iterator<Item = char>` or `&str` and iterate `.chars()` directly. The function only does a forward scan, so no indexing is needed.

- **Pros:** Zero allocation, same logic
- **Cons:** Minor API change
- **Effort:** Small
- **Risk:** Low

## Technical Details

**Affected files:**
- `crates/mika-cli/src/tui/input.rs` — `screen_to_textarea_pos`, `find_char_at_col`

## Acceptance Criteria

- [ ] No `Vec<char>` allocation in mouse drag path
- [ ] Mouse selection still works correctly
- [ ] All tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-09 | Created from code review | Allocation on hot path |
