---
title: "TUI textarea selection - visual rendering and mouse support"
date: "2026-03-09"
category: "ui-bugs"
tags:
  - tui
  - ratatui
  - textarea
  - selection
  - mouse-events
  - rendering
components:
  - crates/mika-cli/src/tui/app.rs
  - crates/mika-cli/src/tui/input.rs
  - crates/mika-cli/src/tui/ui.rs
severity: "moderate"
problem_type: "missing-feature-and-bug"
related_issues:
  - "Ctrl+A/Ctrl+C selection not visually rendered"
  - "Mouse selection not supported in textarea"
  - "screen_to_textarea_pos scroll offset bug"
---

# TUI Textarea Selection: Visual Rendering and Mouse Support

## Problem Symptom

Ctrl+A was wired to `textarea.select_all()` and Ctrl+C to `textarea.yank_text()`, but no visual highlighting appeared. Mouse click-drag in the textarea input area did nothing. When textarea content exceeded 6 visible lines and scrolled, mouse clicks mapped to the wrong logical position.

## Root Cause

The TUI's textarea had three related gaps:

1. **No visual rendering of selections:** `tui-textarea` internally tracked selection state (e.g., via Ctrl+A `select_all()`), but the custom `draw_input()` function bypassed the widget's built-in rendering. It called `wrap_input_with_cursor()` which produced plain, unstyled `Span`s — so selections were invisible even though the underlying state existed.

2. **No mouse selection in textarea:** Mouse event handling (`handle_mouse()`) only supported click-drag selection in the message area. Clicks inside the textarea area were not handled.

3. **Scroll offset ignored in coordinate mapping:** When textarea content exceeded the visible height and scrolled, `screen_to_textarea_pos()` did not account for the scroll offset, meaning mouse clicks mapped to the wrong logical position.

## Solution

### 1. Made `wrap_input_with_cursor()` selection-aware

Extended the function with an `Option<((usize, usize), (usize, usize))>` parameter representing the selection range as `((start_row, start_col), (end_row, end_col))`. When present, per-character boolean selection flags are computed, and `build_selection_line()` produces styled spans with `LightBlue` background / `Black` foreground.

```rust
fn wrap_input_with_cursor(
    text_lines: &[impl AsRef<str>],
    cursor_row: usize, cursor_col: usize,
    width: usize,
    selection: Option<((usize, usize), (usize, usize))>,
) -> WrappedInput
```

### 2. Added `build_selection_line()` helper

Walks characters and selection flags together, grouping consecutive characters with the same selection state into `Span`s — selected characters get highlight style, unselected get `Span::raw`.

### 3. Wired selection into `draw_input()`

Queries `app.textarea.selection_range()` and passes it to `wrap_input_with_cursor()`. Stores computed `scroll_offset` into `app.textarea_scroll_offset` for mouse coordinate mapping.

### 4. Added mouse handling for textarea area

In `handle_mouse()`, `MouseEventKind::Down(Left)` checks `is_in_textarea()` first. On hit, maps screen coordinates via `screen_to_textarea_pos()`, moves cursor with `CursorMove::Jump`, calls `start_selection()`, sets `app.textarea_selecting = true`. `Drag` extends selection. `Up` finalizes.

### 5. Fixed scroll offset in coordinate mapping

`screen_to_textarea_pos()` adds the stored scroll offset to the relative row before walking wrapped lines:

```rust
let rel_row = screen_row.saturating_sub(rect.y) as usize
    + app.textarea_scroll_offset as usize;
```

### 6. Added selection cancellation on keypress

Any keypress (other than Ctrl+C for copy) cancels the textarea selection, matching standard text editor behavior.

## Key Implementation Details

- **Three new fields on `App`:** `textarea_inner_rect: Option<Rect>` (hit-testing), `textarea_scroll_offset: u16` (bridges render/input gap), `textarea_selecting: bool` (drag state machine).

- **Per-character flag approach:** Builds a `Vec<bool>` per text line mapping each character index to selected/unselected. Handles multi-line selections that start and end mid-line, works across wrap boundaries via slice indexing.

- **`screen_to_textarea_pos()` replicates wrapping logic:** Walks all text lines using the same width-based wrapping algorithm as `wrap_input_with_cursor()`, counting display rows until target row, then uses `find_char_at_col()` to resolve column to character index.

- **Mutual exclusion of selection contexts:** Starting a textarea selection cancels any message-area selection (and vice versa), preventing dual-highlight states.

## Prevention Strategies

**Audit the rendering pipeline before customizing widget output.** When wrapping a library widget's content with custom rendering, enumerate every visual state the library tracks internally. Each must be accounted for in the custom renderer or it will silently disappear.

**Treat coordinate mapping as a first-class concern.** Any time screen coordinates are translated to logical positions, build the mapping to explicitly incorporate every transform: vertical scroll offset, line-wrap boundaries, horizontal scroll, and unicode character widths.

**Enforce mutual exclusion across selection mechanisms at the state transition level.** Clear the other region's selection state at the moment a new selection begins — not at render time.

**Prefer delegating to the library widget's selection API over reimplementing selection logic.** Use `selection_range()`, `start_selection()`, `cancel_selection()` as the source of truth. Custom rendering should read from the library's state rather than maintaining a parallel model.

## Best Practices

- **Centralize scroll-offset arithmetic.** Create a single function for (screen_x, screen_y) to (logical_line, logical_col) conversion. Duplicated offset math is the primary source of off-by-one bugs in scrollable views.
- **Use debug assertions for coordinate invariants.** `debug_assert!` checks that logical line indices are within bounds after mapping catch miscalculations immediately.
- **Clear selection on mode transitions.** Any action that changes UI mode should clear all active selections.

## Testing Checklist

- [ ] Select text in textarea; verify highlight covers exactly the selected characters
- [ ] Scroll textarea so content is off-screen above; select in visible area; verify correct mapping
- [ ] Select in message area then click textarea; verify message selection clears
- [ ] Select in textarea then click message area; verify textarea selection clears
- [ ] Select text spanning a soft-wrap boundary; verify highlight continues across visual line break
- [ ] Select text with multi-byte unicode characters; verify boundaries align with character edges
- [ ] With non-zero scroll offset, click at known position; verify cursor lands on expected character
- [ ] Resize terminal with active selection; verify no panic
- [ ] Ctrl+C with no selection and empty input still quits (regression)

## Related Documentation

- **Plan:** `docs/plans/2026-03-09-feat-tui-text-selection-plan.md` — Full implementation plan (GitHub issue #72)
- **Brainstorm:** `docs/brainstorms/2026-03-09-tui-text-selection-brainstorm.md` — Original design decisions
- **Todo #596:** `todos/596-pending-p2-deduplicate-wrapping-logic.md` — Consolidate 4 wrapping implementations (P2)
- **Todo #597:** `todos/597-pending-p3-simplify-selection-highlighting.md` — Merge `build_selection_line()` with `apply_selection_highlight()` (P3)
- **Todo #598:** `todos/598-pending-p3-avoid-vec-char-allocation-in-mouse-drag.md` — Avoid `Vec<char>` allocation per drag event (P3)
- **Todo #599:** `todos/599-complete-p1-textarea-scroll-offset-mouse-mapping.md` — Scroll offset bug fix (P1, complete)
- **Prior art:** `docs/solutions/ui-bugs/tui-persistent-history-and-paste-cursor.md`, `docs/solutions/ui-bugs/tui-shell-like-autocompletion.md`
