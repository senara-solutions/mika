---
status: complete
priority: p3
issue_id: "356"
tags: [code-review, performance, tui]
dependencies: []
---

# Cache TUI Input Height Calculation

## Problem Statement

The input height is recalculated every render frame by calling `visual_line_rows()` on each line of input text. For typical inputs this is negligible, but caching the result and only recalculating on input change or terminal resize would be more efficient.

## Findings

- **Performance Oracle**: Input height recalculated every frame. Could cache and invalidate on text change or resize event.
- **Source**: PR #33 — `crates/mika-cli/src/tui/ui.rs` lines 245-260

## Proposed Solutions

### Solution A: Cache in App state
Add `cached_input_height: Option<u16>` to `App`. Invalidate (set to `None`) on text change or `AppEvent::Resize`. Compute only when `None`.

- **Pros**: Eliminates redundant calculation, straightforward
- **Cons**: Another field on App, invalidation must be correct
- **Effort**: Small
- **Risk**: Low

### Solution B: No action
Current performance is fine for typical input lengths. Optimize only if profiling shows this is a bottleneck.

- **Pros**: No code churn
- **Cons**: Leaves minor inefficiency
- **Effort**: None
- **Risk**: None

## Recommended Action

_To be filled during triage_

## Technical Details

**Affected files:**
- `crates/mika-cli/src/tui/app.rs` — Add cached height field
- `crates/mika-cli/src/tui/ui.rs` — Use cached value in `draw_chat()`

## Acceptance Criteria

- [ ] Input height computed at most once per text change or resize
- [ ] No visual regression in input area sizing
- [ ] All tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from PR #33 code review | Performance oracle identified frame-level recalculation |

## Resources

- PR #33: https://github.com/senara-solutions/mika/pull/33
