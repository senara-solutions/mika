---
status: pending
priority: p2
issue_id: "596"
tags: [code-review, architecture, tui, duplication]
dependencies: []
---

# Deduplicate unicode-width wrapping logic across TUI module

## Problem Statement

There are now 4 independent implementations of the same unicode-width-aware text wrapping algorithm:

| Location | Function | Purpose |
|---|---|---|
| `ui.rs:21` | `visual_line_rows` | Count display rows for height estimation |
| `ui.rs:53` | `wrap_input_with_cursor` | Render textarea with cursor tracking + selection |
| `ui.rs:801` | `find_char_offset_in_wrapped_line` | Map screen position to char offset (messages) |
| `input.rs:32` | `screen_to_textarea_pos` | Map screen position to char offset (textarea) |

All four use the identical wrapping predicate: `col + ch_w > width && col > 0`. A bug fix in one would need to be replicated in all four. This is a maintenance hazard.

## Findings

- Architecture Strategist flagged this as the primary architectural concern
- Code Simplicity Reviewer independently identified the same issue
- The core wrapping walk is ~10 lines; the surrounding code varies by use case

## Proposed Solutions

### Option A: Extract shared wrapping iterator
Create a `WrappingCharIter` or `wrap_walk()` helper that yields `(display_row, display_col, char_idx, byte_offset, ch)` tuples. All four consumers call this iterator instead of reimplementing the walk.

- **Pros:** Single source of truth, easy to test, clear API
- **Cons:** Iterator design adds abstraction; each consumer needs different data from the walk
- **Effort:** Medium
- **Risk:** Low

### Option B: Extract a callback-based walk
Create `fn walk_wrapped_chars(line: &str, width: usize, callback: impl FnMut(...))` that walks characters and calls the callback at each wrap point and character.

- **Pros:** Simpler than iterator, flexible
- **Cons:** Less composable than iterator, closure ergonomics
- **Effort:** Medium
- **Risk:** Low

### Option C: Keep as-is, add tests
Accept the duplication but add shared test fixtures that verify all four implementations agree on the same inputs.

- **Pros:** No refactoring risk, validates correctness
- **Cons:** Doesn't reduce duplication, tests may drift
- **Effort:** Small
- **Risk:** Low

## Technical Details

**Affected files:**
- `crates/mika-cli/src/tui/ui.rs`
- `crates/mika-cli/src/tui/input.rs`

## Acceptance Criteria

- [ ] Single wrapping algorithm implementation used by all 4 call sites
- [ ] All existing tests pass
- [ ] `cargo clippy -p mika-cli` clean

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-09 | Created from code review | 4 copies of wrapping logic identified |
