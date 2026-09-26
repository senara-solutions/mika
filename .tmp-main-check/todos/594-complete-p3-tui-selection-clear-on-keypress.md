---
status: complete
priority: p3
issue_id: "594"
tags: [code-review, ux, tui]
dependencies: []
---

# Clear text selection on any keypress

## Problem Statement

Currently, text selection is only cleared on scroll, resize, or new messages. Pressing regular keys while a selection is active doesn't clear it, which feels inconsistent with standard text editor behavior.

## Findings

- `crates/mika-cli/src/tui/input.rs` — `handle_key_normal` function
- Selection should clear at the top of `handle_key_normal` for any key that isn't Ctrl+C (which handles copy)

## Proposed Solutions

### Solution 1: Clear at top of handle_key_normal (Recommended)
Add `app.selection_state.clear()` early in `handle_key_normal`, before the Ctrl+C check.

**Pros:** Simple, matches user expectations
**Cons:** None
**Effort:** Small
**Risk:** Low

## Acceptance Criteria

- [ ] Any keypress (except Ctrl+C with active selection) clears selection
- [ ] Ctrl+C still copies when selection is active
- [ ] Tests pass

## Work Log

| Date | Action |
|------|--------|
| 2026-03-09 | Created from PR #93 code review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/93
