---
title: "fix: Tab completion cursor jumps to position 0"
type: fix
status: completed
date: 2026-03-02
---

# Fix Tab Completion Cursor Position

## Overview

After tab-completing a slash command, the cursor jumps to position 0 (beginning of input) instead of being placed at the end of the completed text.

## Problem Statement

The `set_textarea()` function in `input.rs:387` constructs a new `TextArea` from the completed text but never positions the cursor. `TextArea::from(...)` defaults cursor to (0, 0), so after completion the cursor is at the beginning instead of the end.

## Root Cause

```rust
fn set_textarea(app: &mut App<'_>, text: &str) {
    app.textarea = tui_textarea::TextArea::from(vec![text.to_string()]);
    app.textarea.set_cursor_line_style(ratatui::style::Style::default());
    // BUG: cursor is at (0, 0) — missing move to end
}
```

## Proposed Solution

Add `app.textarea.move_cursor(tui_textarea::CursorMove::End)` after construction:

```rust
fn set_textarea(app: &mut App<'_>, text: &str) {
    app.textarea = tui_textarea::TextArea::from(vec![text.to_string()]);
    app.textarea.set_cursor_line_style(ratatui::style::Style::default());
    app.textarea.move_cursor(tui_textarea::CursorMove::End);
}
```

## Acceptance Criteria

- [x] After tab-completing a command, cursor is at end of completed text
- [x] After tab-completing with args space, cursor is after the space
- [x] After enter-completing a command, cursor is at end
- [x] All existing tests pass
