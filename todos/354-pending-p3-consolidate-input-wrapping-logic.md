---
status: pending
priority: p3
issue_id: "354"
tags: [code-review, performance, quality, tui]
dependencies: []
---

# Consolidate Input Wrapping Logic

## Problem Statement

The `draw_input()` function in `ui.rs` (~70 lines) duplicates the character-width-aware iteration logic that `visual_line_rows()` already performs. This means every render frame iterates over input text twice — once for height calculation and once for rendering. While not a performance issue at current scale, consolidating would improve maintainability and reduce cognitive load.

## Findings

- **Performance Oracle**: `draw_input()` re-iterates what `visual_line_rows()` already computes. Could consolidate into a single pass that returns both display lines and cursor position.
- **Code Simplicity Reviewer**: `draw_input()` is complex at ~70 lines. Consider extracting the wrapping logic into a shared helper.
- **Source**: PR #33 — `crates/mika-cli/src/tui/ui.rs` lines 19-38 and 300-367

## Proposed Solutions

### Solution A: Extract shared wrapping helper
Create a `wrap_text_with_cursor()` function that returns `(Vec<Line>, u16, u16)` (display lines, cursor_x, cursor_y). Both `visual_line_rows()` and `draw_input()` delegate to it.

- **Pros**: Single source of truth, testable, reduces draw_input complexity
- **Cons**: Slight API overhead; visual_line_rows callers don't need cursor info
- **Effort**: Small
- **Risk**: Low

### Solution B: Cache computed display lines
Compute wrapped lines once per input change (not per frame), store in App state, reuse in both height calc and rendering.

- **Pros**: Eliminates redundant computation entirely, best performance
- **Cons**: Adds cache invalidation concern; must invalidate on input change, resize
- **Effort**: Medium
- **Risk**: Low-Medium (cache staleness bugs possible)

## Recommended Action

_To be filled during triage_

## Technical Details

**Affected files:**
- `crates/mika-cli/src/tui/ui.rs` — `visual_line_rows()` and `draw_input()`

## Acceptance Criteria

- [ ] Single wrapping pass per render (or cached)
- [ ] `draw_input()` reduced to < 40 lines
- [ ] All existing tests pass
- [ ] Unicode and CJK wrapping still correct

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from PR #33 code review | Performance and simplicity agents both flagged this |

## Resources

- PR #33: https://github.com/senara-solutions/mika/pull/33
