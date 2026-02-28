---
title: "TUI Input History, Mouse Scroll, and Text Wrapping"
date: 2026-02-28
category: ui-bugs
tags: [tui, ratatui, input-history, mouse-scroll, text-wrapping, unicode, keybindings]
component: mika-cli
modules: [tui/app, tui/input, tui/ui, tui/event, commands/chat]
severity: medium
resolution_time: single-session
pr: 33
---

# TUI Input History, Mouse Scroll, and Text Wrapping

## Problem Statement

The Mika TUI lacked three UX features expected of modern terminal applications:

1. **No input history navigation** — Users had to retype previous messages. The existing Up/Down history only triggered when input was completely empty, so users couldn't access history while composing multi-line text.

2. **No mouse scroll support** — Scrolling conversation history required keyboard-only navigation (PageUp/PageDown). No mouse wheel support existed, and when new messages arrived while the user had scrolled up, the view would unconditionally jump to the bottom, losing their scroll context.

3. **Input text overflow** — Long input text scrolled left instead of wrapping. The tui-textarea library (v0.7) has no wrapping API (open issue since 2022). The input height calculation also used byte length instead of Unicode display width, causing incorrect sizing for non-ASCII text.

## Root Cause

- **History**: Navigation was gated on `input_text().is_empty()` rather than cursor position, preventing multi-line history access.
- **Mouse**: `EnableMouseCapture` was never called during terminal setup, so crossterm never emitted mouse events.
- **Wrapping**: tui-textarea renders its own widget without wrapping support. The height calculation used `.len()` (byte length) instead of character display widths.
- **Auto-scroll**: `scroll_offset = 0` was set unconditionally on new messages, with no check for whether the user had scrolled up.

## Solution

### 1. InputHistory Struct (app.rs)

Replaced scattered `Vec<String>` + `Option<usize>` with an encapsulated struct:

```rust
pub struct InputHistory {
    entries: Vec<String>,
    index: Option<usize>,
    saved_draft: Option<String>,
}

const HISTORY_MAX_SIZE: usize = 500;
```

Key behaviors:
- **Draft saving**: On first `previous()` call, saves current input. Restored when cycling past newest entry.
- **Boundary clamping**: At oldest entry, `previous()` stays (no wrap-around). Past newest, `next()` restores draft.
- **FIFO eviction**: 500-entry cap with oldest-first removal.
- **Push resets**: `push()` clears navigation state unconditionally.

### 2. Cursor-Position-Aware History (input.rs)

Replaced empty-input check with cursor position detection:

```rust
// Up triggers history when cursor is on the first row
if key.code == KeyCode::Up && app.textarea.cursor().0 == 0 {
    app.history_previous();
    return;
}
// Down triggers history when cursor is on the last row
if key.code == KeyCode::Down
    && app.textarea.cursor().0 == app.textarea.lines().len().saturating_sub(1)
{
    app.history_next();
    return;
}
```

### 3. Mouse Event Pipeline (event.rs, chat.rs, input.rs)

Added `AppEvent::Mouse(MouseEvent)` variant, `EnableMouseCapture` in terminal setup, and a whitelist-only handler:

```rust
pub fn handle_mouse(app: &mut App<'_>, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => app.scroll_up(3),
        MouseEventKind::ScrollDown => app.scroll_down(3),
        _ => {} // Ignore clicks, drags — terminal text selection still works
    }
}
```

### 4. Conditional Auto-Scroll (app.rs)

```rust
pub fn auto_scroll_to_bottom(&mut self) {
    if self.scroll_offset == 0 {
        return; // Already at bottom — no-op
    }
    self.has_new_message = true; // Set flag instead of jumping
    self.needs_redraw = true;
}
```

The `has_new_message` flag is cleared when the user scrolls back to bottom (`scroll_down()` checks `scroll_offset == 0`).

### 5. Unicode-Width-Aware Text Wrapping (ui.rs)

Custom rendering replaces tui-textarea's widget:

```rust
pub(crate) fn visual_line_rows(line: &str, width: usize) -> usize {
    let mut rows = 1;
    let mut col = 0;
    for ch in line.chars() {
        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col + ch_w > width && col > 0 {
            rows += 1;
            col = ch_w;
        } else {
            col += ch_w;
        }
    }
    rows
}
```

The `wrap_input_with_cursor()` helper builds display lines and tracks cursor position through wrapping. The textarea still handles editing — only the visual rendering is replaced.

### 6. Keyboard Shortcuts (input.rs)

- **Ctrl+Up/Down**: Scroll conversation by 1 line (fine control while typing)
- **PageUp/PageDown**: Already existed for fast scrolling

Priority order: autocomplete > Ctrl+modifiers > cursor-position history > textarea passthrough.

### 7. Scroll Indicators (ui.rs draw_footer)

Footer shows context-aware indicators when scrolled up:
- "↑ scrolled" (dark gray) — user has scrolled up, at rest
- "↓ new messages" (yellow bold) — new content arrived while scrolled up

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| History struct vs inline state | Dedicated `InputHistory` | Encapsulates navigation logic, enables 10+ unit tests |
| History trigger condition | Cursor row position | Allows history access during multi-line editing |
| Text wrapping strategy | Manual unicode-width rendering | tui-textarea has no wrapping API; custom approach is minimal and reversible |
| Auto-scroll behavior | Conditional with flag | Preserves user scroll context; "↓ new messages" badge provides feedback |
| Mouse scroll amount | 3 lines per event | Standard terminal convention |
| History max size | 500 entries (const) | Reasonable session limit; prevents unbounded growth |

## Prevention Strategies

### For Future TUI Work

1. **Always use `unicode-width` for display calculations** — Never use `.len()` for width estimation. Test with CJK characters and emoji.

2. **Encapsulate related state** — Group fields that change together into structs with invariant-preserving methods (e.g., `InputHistory` instead of separate `Vec` + `Option<usize>`).

3. **Event pipeline checklist** for new input types:
   - [ ] Terminal feature flag (e.g., `EnableMouseCapture`)
   - [ ] `AppEvent` enum variant
   - [ ] Event reader match arm
   - [ ] Main loop dispatch
   - [ ] Handler function in `input.rs`

4. **Separate user actions from auto-behaviors** — Use conditional methods like `auto_scroll_to_bottom()` instead of unconditional state resets.

5. **Match enable/disable pairs** — `EnableMouseCapture` must pair with `DisableMouseCapture` on exit (including panic hooks).

### Gotchas

- **tui-textarea wrapping**: No built-in support as of v0.7. Check changelog before upgrading.
- **Byte vs character indexing**: Use `char_indices()` for safe UTF-8 slicing. Never mix `char_idx` with byte offsets.
- **Keybinding priority**: Ctrl+modifiers must be checked before plain Up/Down to prevent history from stealing Ctrl+Up.
- **Placeholder consistency**: "Type a message..." is set in multiple places (`reset_textarea()`, `history_previous()`, `history_next()`). Keep in sync.

## Testing

14 unit tests added covering:
- **InputHistory** (10 tests): empty history, push/previous, boundary clamping, draft save/restore, max size cap, empty string rejection, reset, navigation state
- **visual_line_rows** (5 tests): empty string, short text, exact width, wrapping, zero width

Key test patterns:
```rust
#[test]
fn test_history_next_restores_draft() {
    let mut h = InputHistory::new();
    h.push("entry1".to_string());
    h.push("entry2".to_string());
    assert_eq!(h.previous("my draft").unwrap(), "entry2");
    assert_eq!(h.previous("my draft").unwrap(), "entry1");
    assert_eq!(h.next().unwrap(), "entry2");
    assert_eq!(h.next().unwrap(), "my draft"); // Draft restored
}
```

## Files Modified

| File | Changes |
|------|---------|
| `crates/mika-cli/src/tui/app.rs` | `InputHistory` struct, `auto_scroll_to_bottom()`, `has_new_message` flag, 14 tests |
| `crates/mika-cli/src/tui/event.rs` | `AppEvent::Mouse` variant, mouse event dispatch |
| `crates/mika-cli/src/tui/input.rs` | `handle_mouse()`, Ctrl+Up/Down, cursor-position history |
| `crates/mika-cli/src/tui/ui.rs` | `visual_line_rows()`, `wrap_input_with_cursor()`, scroll indicators |
| `crates/mika-cli/src/commands/chat.rs` | `EnableMouseCapture`, mouse event loop dispatch |
| `Cargo.toml` | Added `unicode-width = "0.2"` workspace dependency |

## Related Documentation

- [TUI Scroll Clipping Fix](tui-scroll-clipping-word-wrap-estimation.md) — Earlier fix for `Paragraph::line_count()` vs manual estimation
- [TUI Slash Commands and Image Paste](../feature-implementation/tui-slash-commands-web-search-image-paste.md) — Previous TUI feature additions
- [Skill Hallucination and Scroll Fix](../logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md) — Earlier scroll calculation fix
- Plan: `docs/plans/2026-02-28-feat-tui-input-history-mouse-scroll-text-wrap-plan.md`
- PR: #33
