---
title: "feat: TUI input history, mouse scroll, and text wrapping"
type: feat
status: completed
date: 2026-02-28
---

# TUI Input History, Mouse Scroll, and Text Wrapping

## Overview

Improve the Mika TUI with three UX enhancements: shell-like input history navigation via Up/Down arrows, mouse scroll support for the conversation pane, and proper text wrapping in the input field. Also adds Ctrl+Up/Down for scrolling conversation from the input field and visual indicators when scrolled up.

## Problem Statement / Motivation

The TUI currently lacks standard terminal UX patterns that users expect:
1. **Input history** only works when the input is completely empty — no cursor-position-aware navigation
2. **No mouse scroll** — users must use PageUp/PageDown to review conversation history
3. **Input text overflows** horizontally instead of wrapping, making long messages hard to compose
4. **No Ctrl+Up/Down** to scroll conversation while typing
5. **No visual feedback** when scrolled up from the bottom of conversation

## Proposed Solution

### Design Decisions (from SpecFlow analysis)

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Text wrapping approach | Manual line-splitting before passing to tui-textarea | tui-textarea 0.7 has no wrapping API (open issue since 2022). Fork `tui-textarea-2` is unvetted. Manual approach is minimal and reversible. |
| Mouse capture | Always-on | Shift+click still passes through to terminal for text selection in most emulators. Document this. |
| Draft saving on history | Yes — save current input, restore when cycling past newest | Without this, cursor-position-aware history causes data loss on partially typed messages. |
| History trigger condition | Up when `cursor().0 == 0`, Down when `cursor().0 == last_row` | Matches multi-line editor conventions. Column position is irrelevant. |
| Mouse scroll amount | 3 lines per wheel event | Standard terminal convention. |
| Ctrl+Up/Down scroll amount | 1 line per event | Fine-grained control while typing. |
| Scroll indicator location | Footer bar, right-aligned | Minimal visual impact, always visible. |
| New message indicator | Show "new messages" badge in footer when scrolled up and new content arrives | Combined with scroll indicator for simplicity. |
| scroll_offset clamping | Clamp on render, not on scroll events | Avoids needing to pass render-time info back to App state. The current approach works visually. |
| History size limit | Cap at 500 entries | Reasonable for a chat session, prevents unbounded growth. |
| History persistence | In-memory only (no cross-session persistence) | Keep scope minimal. Can add file persistence later. |

## Technical Approach

### Key Files

| File | Changes |
|------|---------|
| `crates/mika-cli/src/tui/app.rs` | `InputHistory` struct, `saved_draft`, `has_new_message`, `user_scrolled_up` flag, conditional auto-scroll |
| `crates/mika-cli/src/tui/input.rs` | Cursor-position-aware history, Ctrl+Up/Down, mouse event handler |
| `crates/mika-cli/src/tui/event.rs` | `AppEvent::Mouse` variant, handle `CrosstermEvent::Mouse` |
| `crates/mika-cli/src/tui/ui.rs` | Text wrapping in input, scroll indicator in footer, new-message badge |
| `crates/mika-cli/src/commands/chat.rs` | `EnableMouseCapture` in setup, `Mouse` event dispatch in main loop |

### Architecture

```
┌─────────────────────────────────────────────┐
│  Terminal Setup (chat.rs)                    │
│  + EnableMouseCapture                        │
├─────────────────────────────────────────────┤
│  EventReader (event.rs)                      │
│  + CrosstermEvent::Mouse → AppEvent::Mouse   │
├─────────────────────────────────────────────┤
│  Main Loop (chat.rs)                         │
│  + AppEvent::Mouse match arm                 │
├─────────────────────────────────────────────┤
│  Input Handler (input.rs)                    │
│  + handle_mouse() — scroll dispatch          │
│  + Ctrl+Up/Down — conversation scroll        │
│  + Cursor-aware Up/Down — history navigation │
├─────────────────────────────────────────────┤
│  App State (app.rs)                          │
│  + InputHistory { entries, index, draft }    │
│  + has_new_message: bool                     │
│  + auto_scroll_to_bottom() conditional       │
├─────────────────────────────────────────────┤
│  UI Rendering (ui.rs)                        │
│  + draw_input: pre-wrap long lines           │
│  + draw_footer: scroll/new-message indicator │
└─────────────────────────────────────────────┘
```

## Implementation Phases

### Phase 1: Input History Enhancement

**Files:** `app.rs`, `input.rs`

#### 1.1 Extract InputHistory struct (`app.rs`)

```rust
// crates/mika-cli/src/tui/app.rs
pub struct InputHistory {
    entries: Vec<String>,       // Sent messages, oldest first
    index: Option<usize>,       // None = not browsing, Some(i) = position
    saved_draft: Option<String>, // Current input saved when entering history
    max_size: usize,            // Cap at 500
}

impl InputHistory {
    pub fn new() -> Self { ... }
    pub fn push(&mut self, entry: String) { ... }  // Add entry, trim to max_size
    pub fn previous(&mut self, current_input: &str) -> Option<&str> { ... }
    pub fn next(&mut self) -> HistoryResult { ... }  // Returns Entry(str) or Draft(str) or Empty
    pub fn reset(&mut self) { ... }  // Reset index and draft on send
}
```

- Move `input_history: Vec<String>` and `history_index: Option<usize>` into `InputHistory`
- Add `saved_draft: Option<String>` — saved on first `previous()` call when `index.is_none()`
- `previous()`: If `index.is_none()`, save `current_input` as draft, set `index = Some(entries.len() - 1)`. If `index == Some(0)`, stay at 0 (clamp). Otherwise decrement.
- `next()`: If `index == Some(entries.len() - 1)`, set `index = None`, return saved draft (or empty if no draft). If `index.is_none()`, no-op.
- `push()`: Add entry, if `entries.len() > max_size` remove oldest. Reset index and draft.

**Acceptance criteria:**
- [x] `InputHistory` struct with push/previous/next/reset
- [x] Draft saved on first Up press, restored when cycling past newest
- [x] Capped at 500 entries
- [x] Unit tests for all navigation paths

#### 1.2 Cursor-position-aware history triggers (`input.rs`)

Replace the current empty-input check with cursor position detection:

```rust
// crates/mika-cli/src/tui/input.rs — in handle_key_normal()

// Current (lines 141-149):
KeyCode::Up if app.input_text().is_empty() => { app.history_previous(); }
KeyCode::Down if app.input_text().is_empty() => { app.history_next(); }

// New:
KeyCode::Up if app.textarea.cursor().0 == 0 => {
    app.history_previous();  // cursor on first row → history
}
KeyCode::Down if app.textarea.cursor().0 == app.textarea.lines().len().saturating_sub(1) => {
    app.history_next();  // cursor on last row → history
}
// Otherwise, Up/Down fall through to app.textarea.input(key) for cursor movement
```

**Acceptance criteria:**
- [x] Up triggers history when cursor is on row 0
- [x] Down triggers history when cursor is on the last row
- [x] Up/Down move cursor normally on intermediate rows of multi-line input
- [x] Autocomplete popup still takes priority (existing `if app.autocomplete.visible` guard)

### Phase 2: Mouse Scroll Support

**Files:** `event.rs`, `chat.rs`, `input.rs`, `app.rs`

#### 2.1 Event plumbing (`event.rs`, `chat.rs`)

```rust
// crates/mika-cli/src/tui/event.rs — AppEvent enum
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),  // NEW
    Paste(String),
    Tick,
    Resize,
}

// In event reader thread — handle CrosstermEvent::Mouse
CrosstermEvent::Mouse(mouse) => {
    let _ = sender.send(AppEvent::Mouse(mouse));
}
```

```rust
// crates/mika-cli/src/commands/chat.rs — terminal setup (line 241)
execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, EnableMouseCapture)?;

// Main loop — new match arm
Some(AppEvent::Mouse(mouse)) => {
    input::handle_mouse(&mut app, mouse);
    app.needs_redraw = true;
}
```

#### 2.2 Mouse scroll handler (`input.rs`)

```rust
// crates/mika-cli/src/tui/input.rs
pub fn handle_mouse(app: &mut App<'_>, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            app.scroll_up(3);
        }
        MouseEventKind::ScrollDown => {
            app.scroll_down(3);
        }
        _ => {} // Ignore clicks, drags, etc.
    }
}
```

#### 2.3 Conditional auto-scroll (`app.rs`)

Replace unconditional `self.scroll_offset = 0` with a method:

```rust
// crates/mika-cli/src/tui/app.rs
pub fn auto_scroll_to_bottom(&mut self) {
    if self.scroll_offset == 0 {
        // Already at bottom — stay at bottom (no-op, scroll_offset is already 0)
        return;
    }
    // User has scrolled up — don't auto-scroll, set new-message flag
    self.has_new_message = true;
    self.needs_redraw = true;
}
```

Call `auto_scroll_to_bottom()` in places that currently set `scroll_offset = 0` for agent responses:
- `app.rs` line ~388 (reveal complete)
- Anywhere new assistant content is appended during streaming

Keep the unconditional `scroll_offset = 0` for user-initiated actions:
- After sending a message (`app.rs` line 271) — user wants to see the response
- After `/clear` command
- After agent switch

When user scrolls down to bottom (`scroll_offset == 0`), clear `has_new_message`:

```rust
pub fn scroll_down(&mut self, amount: usize) {
    self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    if self.scroll_offset == 0 {
        self.has_new_message = false;
    }
    self.needs_redraw = true;
}
```

**Acceptance criteria:**
- [x] Mouse scroll up/down scrolls conversation by 3 lines per event
- [x] `EnableMouseCapture` called during terminal setup
- [x] `AppEvent::Mouse` variant added and handled in event reader
- [x] Auto-scroll only when user was already at bottom
- [x] `has_new_message` flag set when content arrives while scrolled up
- [x] `has_new_message` cleared when user scrolls back to bottom

### Phase 3: Ctrl+Up/Down and Scroll Indicator

**Files:** `input.rs`, `ui.rs`

#### 3.1 Ctrl+Up/Down keybindings (`input.rs`)

Add before the history check in `handle_key_normal()`:

```rust
// Ctrl+Up/Down — scroll conversation without leaving input
KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
    app.scroll_up(1);
    return;  // Don't fall through to history or textarea
}
KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
    app.scroll_down(1);
    return;
}
```

**Important:** These must be checked BEFORE the cursor-position history check to take priority.

#### 3.2 Scroll indicator in footer (`ui.rs`)

In `draw_footer()`, add a right-aligned indicator when scrolled up:

```rust
// crates/mika-cli/src/tui/ui.rs — in draw_footer()
if app.scroll_offset > 0 {
    if app.has_new_message {
        // Show "↓ new messages" in yellow
        spans.push(Span::styled(" ↓ new messages ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    } else {
        // Show "↑ scrolled" in dark gray
        spans.push(Span::styled(" ↑ scrolled ", Style::default().fg(Color::DarkGray)));
    }
}
```

**Acceptance criteria:**
- [x] Ctrl+Up scrolls conversation up 1 line
- [x] Ctrl+Down scrolls conversation down 1 line
- [x] Footer shows "↑ scrolled" when `scroll_offset > 0`
- [x] Footer shows "↓ new messages" when scrolled up and new content arrived
- [x] Indicators disappear when scrolled back to bottom

### Phase 4: Input Text Wrapping

**Files:** `ui.rs`, `app.rs`

#### 4.1 Pre-wrap input text for rendering (`ui.rs`)

Since tui-textarea 0.7 does not support visual wrapping, implement manual line-splitting. The approach: intercept the textarea content before rendering and re-flow long lines to fit the available width.

**Strategy:** Override how the textarea is displayed by wrapping its content at the widget boundary. We'll modify the textarea's lines in-place before rendering and restore them after, OR we use a separate display approach.

After analysis, the cleanest approach is to set the textarea to use `hard_wrap` mode if available, or to pre-split lines at the available width before each render:

```rust
// crates/mika-cli/src/tui/ui.rs — in draw_input()
// Before rendering, wrap long lines to fit the available width
let input_width = input_area.width.saturating_sub(2) as usize; // subtract prompt "> "
// Let tui-textarea handle rendering — it scrolls horizontally by default
// To simulate wrapping, we insert visual newlines into the textarea content
```

**Alternative (simpler) approach:** Accept that tui-textarea scrolls horizontally and instead fix the **input height calculation** to account for wrapped visual width, then render the textarea content as a `Paragraph` with `Wrap` instead of using the textarea widget directly for display. The textarea still handles editing; we just render differently.

**Recommended approach:** Use tui-textarea's `set_line_number_style(None)` and accept horizontal scrolling as a known limitation for now. Instead, focus on making the input height calculation correct so the textarea grows taller as the user types more lines (via Enter/Shift+Enter). The key fix is the height calculation in `draw()`:

```rust
// crates/mika-cli/src/tui/ui.rs — fix input height calculation (lines 20-34)
// Current buggy estimation:
//   lines.iter().map(|l| (l.len() / available_width) + 1).sum()
// Fix: use unicode_width for accurate character width measurement
let content_lines: usize = app.textarea.lines().iter().map(|l| {
    let w = unicode_width::UnicodeWidthStr::width(l.as_str());
    if w == 0 { 1 } else { (w / available_width) + 1 }
}).sum();
let input_height = content_lines.clamp(1, 6) + 2; // +2 for borders/padding
```

Then, to actually enable visual wrapping of text in the input widget, we set `LineBreak::Wrap` on the textarea if the API supports it, or we hook into the textarea's rendering to force wrapping.

**Final decision:** Since tui-textarea genuinely does not support wrapping and building a custom widget is out of scope, we will:
1. Fix the height calculation to use `unicode_width` (resolving the documented gotcha)
2. Pre-split long lines at word boundaries when the user types past the available width, inserting actual newlines into the textarea content — this makes the textarea "think" the user typed multiple lines
3. Track the original logical line so that `input_text()` still returns the intended content without artificial newlines

```rust
// crates/mika-cli/src/tui/app.rs
/// Wraps the current textarea content to fit within the given width.
/// Inserts newlines at word boundaries for lines exceeding the width.
/// Called on each keystroke that modifies content.
pub fn rewrap_input(&mut self, available_width: usize) {
    let original = self.input_text();
    let wrapped_lines = textwrap::wrap(&original, available_width);
    // Only rebuild textarea if wrapping changed
    let new_lines: Vec<String> = wrapped_lines.iter().map(|l| l.to_string()).collect();
    if new_lines != self.textarea.lines().to_vec() {
        // Save cursor position as character offset, rebuild, restore
        let char_offset = self.cursor_char_offset();
        self.textarea = TextArea::from(new_lines);
        self.restore_cursor_from_offset(char_offset);
        self.textarea.set_cursor_line_style(Style::default());
        self.textarea.set_placeholder_text("Type a message...");
    }
}
```

**Note:** This approach has cursor position management complexity. If it proves too fragile, fall back to horizontal scrolling (the status quo) and file a separate issue to track proper wrapping support.

**Acceptance criteria:**
- [x] Long input text wraps visually within the input box
- [x] Cursor navigation still works correctly after wrapping
- [x] `input_text()` returns the logical content (not the wrapped version)
- [x] Input height grows correctly as content wraps (up to 6-line cap)
- [x] If wrapping approach proves too fragile, document limitation and defer

### Phase 5: Testing

**Files:** `app.rs` (inline tests), `input.rs` (inline tests)

#### 5.1 InputHistory unit tests

```rust
#[cfg(test)]
mod tests {
    // Test: push entries and navigate with previous/next
    // Test: cycling past oldest stays at oldest
    // Test: cycling past newest restores draft
    // Test: draft is saved on first previous() call
    // Test: push() resets index and draft
    // Test: max_size cap trims oldest entries
    // Test: empty history — previous() returns None
}
```

#### 5.2 Cursor-position history trigger tests

```rust
#[cfg(test)]
mod tests {
    // Test: single-line input, cursor at (0, 0) — Up triggers history
    // Test: single-line input, cursor at (0, 5) — Up triggers history (row 0)
    // Test: multi-line input, cursor at (0, x) — Up triggers history
    // Test: multi-line input, cursor at (1, x) — Up moves cursor (not history)
    // Test: multi-line input, cursor at (last_row, x) — Down triggers history
}
```

#### 5.3 Scroll behavior tests

```rust
#[cfg(test)]
mod tests {
    // Test: scroll_up increases scroll_offset
    // Test: scroll_down decreases scroll_offset, clamps at 0
    // Test: scroll_down to 0 clears has_new_message
    // Test: auto_scroll_to_bottom when already at bottom — no-op
    // Test: auto_scroll_to_bottom when scrolled up — sets has_new_message
}
```

**Acceptance criteria:**
- [x] All InputHistory navigation paths covered
- [x] Cursor-position detection logic tested for single/multi-line inputs
- [x] Scroll state transitions tested
- [x] `cargo test` passes with no regressions

## Dependencies & Risks

| Risk | Mitigation |
|------|------------|
| tui-textarea has no wrapping API | Manual pre-wrapping with `textwrap` crate. Fallback: defer and document limitation. |
| Mouse capture breaks terminal text selection | Document Shift+click passthrough. Most modern terminals support this. |
| Ctrl+Up/Down intercepted by some terminals | These are supplementary — PageUp/PageDown still work. Not a blocker. |
| Pre-wrapping cursor management fragile | If cursor jumps or behaves incorrectly, fall back to horizontal scrolling. |
| `textwrap` dependency | Lightweight, well-maintained crate. Minimal risk. |

## New Dependencies

- `textwrap` crate — for word-boundary line splitting in the input wrapping implementation
- `unicode-width` — may already be a transitive dependency via ratatui; verify before adding

## Acceptance Criteria

### Functional Requirements

- [x] Up arrow cycles through input history when cursor is on first row
- [x] Down arrow cycles through input history when cursor is on last row
- [x] Up/Down move cursor normally on intermediate rows
- [x] Draft input saved when entering history, restored when cycling past newest
- [x] History capped at 500 entries
- [x] Mouse scroll up/down scrolls conversation by 3 lines
- [x] Auto-scroll to bottom on new message only when user was at bottom
- [x] "↑ scrolled" indicator in footer when scrolled up
- [x] "↓ new messages" indicator when scrolled up and new content arrives
- [x] Ctrl+Up/Down scrolls conversation by 1 line from input field
- [x] Long input text wraps within the input box (or documented deferral)
- [x] PageUp/PageDown continue to work (existing, scroll by 5)

### Non-Functional Requirements

- [x] No performance regression in the 30ms tick render loop
- [x] No panics on UTF-8 multi-byte characters in input/history
- [x] Mouse events handled without blocking the event loop

## References

### Internal References

- Input handling: `crates/mika-cli/src/tui/input.rs:65-158`
- App state & scroll: `crates/mika-cli/src/tui/app.rs:69,409-456`
- Event reader: `crates/mika-cli/src/tui/event.rs:7-47`
- UI rendering: `crates/mika-cli/src/tui/ui.rs:17-255`
- Terminal setup: `crates/mika-cli/src/commands/chat.rs:239-243`
- Footer rendering: `crates/mika-cli/src/tui/ui.rs:257-331`

### Learnings Applied

- `docs/solutions/ui-bugs/tui-scroll-clipping-word-wrap-estimation.md` — Don't manually estimate wrapped line counts
- `docs/solutions/logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md` — Use visual rows after word-wrapping
- `docs/solutions/code-review-workflow/mika-cli-21-findings-parallel-resolution.md` — UTF-8 boundary-safe indexing
