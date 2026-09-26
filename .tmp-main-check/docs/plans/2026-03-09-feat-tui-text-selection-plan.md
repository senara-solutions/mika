---
title: "feat: TUI text selection and copy"
type: feat
status: completed
date: 2026-03-09
origin: docs/brainstorms/2026-03-09-tui-text-selection-brainstorm.md
---

# feat: TUI Text Selection and Copy

## Overview

Add mouse-based text selection and clipboard copy to the TUI conversation panel. Users can click-and-drag to highlight text within a single message, then Ctrl+C to copy it to the system clipboard. This requires a foundational refactor: replacing the monolithic single-Paragraph message renderer with per-message widget rendering, giving each message its own screen coordinates and enabling click-target identification.

**Issue:** https://github.com/senara-solutions/mika/issues/72

## Problem Statement

The TUI currently provides no text selection mechanism. Users must rely on terminal emulator selection, which captures full-width content including UI chrome (borders, padding) and is suppressed by `EnableMouseCapture`. This makes it impossible to cleanly copy code snippets, responses, or conversation excerpts.

## Proposed Solution

A two-phase approach (see brainstorm: `docs/brainstorms/2026-03-09-tui-text-selection-brainstorm.md`):

1. **Phase 1: Per-message widget refactor** — Break the monolithic `Paragraph` in `draw_messages` into individual message widgets with known screen rectangles. Rework scroll to operate on per-message heights. This is the riskiest change and should land as a standalone refactor.

2. **Phase 2: Selection and copy** — Add mouse click-and-drag selection state machine, visual highlight rendering, and Ctrl+C copy behavior on top of the per-message architecture.

## Technical Approach

### Architecture

#### Per-Message Rendering Model

Replace the current flat `Vec<Line<'static>>` approach in `draw_messages` (`ui.rs:190-330`) with a model where each message is rendered as its own `Paragraph` widget into a calculated sub-rect of the messages area.

**New data structures on `App`:**

```rust
/// Screen rectangle and metadata for a rendered message
struct MessageLayout {
    /// Index into app.messages
    message_idx: usize,
    /// Wrapped line count at current terminal width
    wrapped_lines: usize,
    /// Screen rect where this message was rendered (set during draw)
    screen_rect: Option<Rect>,
}

/// Cached layout for the messages panel
struct MessagesLayout {
    /// Per-message layout info, in display order
    entries: Vec<MessageLayout>,
    /// Sum of all wrapped_lines (includes spacer lines between messages)
    total_lines: usize,
    /// Terminal width used to compute this layout (invalidate on change)
    computed_at_width: u16,
    /// Message count when last computed (invalidate on new messages)
    computed_at_count: usize,
}
```

**Scroll integration:** The inverted offset model (`scroll_offset` counts from bottom) is preserved. `total_lines` replaces the monolithic `paragraph.line_count(width)`. Visible message range is determined by binary search on cumulative line offsets. Partially visible messages at the top/bottom viewport edges use `Paragraph::scroll()` for clipping.

**Layout invalidation triggers:**
- Terminal resize (`AppEvent::Resize`) → recompute all
- New message added → append + update total
- Content streaming (`pending_response` change) → recompute last entry
- Agent status transition → recompute (thinking indicator appears/disappears)
- Team dashboard toggle → recompute (width changes)

#### Selection State Machine

```rust
/// Text position within a message
#[derive(Clone, Copy)]
struct TextPosition {
    /// Line index within the message's rendered lines
    line: usize,
    /// Character offset within the line (by unicode width)
    char_offset: usize,
}

enum SelectionState {
    /// No selection active
    None,
    /// Mouse button is held, drag in progress
    Dragging {
        message_idx: usize,
        anchor: TextPosition,
        current: TextPosition,
    },
    /// Selection complete (mouse released)
    Selected {
        message_idx: usize,
        start: TextPosition,
        end: TextPosition,
    },
}
```

**State transitions:**

| Event | From `None` | From `Dragging` | From `Selected` |
|-------|-------------|-----------------|-----------------|
| MouseDown in message | → `Dragging(msg, pos, pos)` | — | → `Dragging(new_msg, pos, pos)` |
| MouseDown outside messages | no-op | → `None` | → `None` |
| MouseDrag | no-op | update `current` (clamp to message bounds) | no-op |
| MouseUp | no-op | → `Selected(msg, start, end)` (normalize direction) | no-op |
| Scroll (any) | no-op | → `None` | → `None` |
| Resize | no-op | → `None` | → `None` |
| Ctrl+C | quit | → `None` (cancel) | copy text → `None` |
| Content change (streaming) | no-op | → `None` | → `None` |
| Status transition | no-op | → `None` | → `None` |
| Click (Down+Up same pos) | no-op | → `None` | → `None` |

#### Screen-to-Text Hit Testing

The core technical challenge: mapping screen `(column, row)` to `(message_idx, TextPosition)`.

**Approach:** During `draw_messages`, after computing which messages are visible and their screen rects, build a hit-test map. For each visible message, walk the rendered `Line<'static>` objects using `unicode_width::UnicodeWidthStr` to map screen columns to character offsets, accounting for word wrapping at the available width. This replicates the logic already used in `wrap_input_with_cursor` (`ui.rs:19-38`) but generalized for multi-Span Lines.

```rust
/// Map screen (col, row) to a text position within a visible message.
/// Returns None if the position is outside any message.
fn hit_test(
    col: u16,
    row: u16,
    messages_layout: &MessagesLayout,
    messages_inner_rect: Rect,
    scroll_offset: usize,
) -> Option<(usize, TextPosition)> { ... }
```

**Unicode correctness:** Use `unicode_width::UnicodeWidthChar` for per-character width (CJK = 2, most Latin = 1, zero-width combiners = 0). This matches ratatui's internal width calculation.

#### Selection Highlight Rendering

Apply reverse-video styling to selected Spans during rendering. When a message has an active selection, split its `Span` objects at the selection boundaries and apply `Style::default().bg(Color::White).fg(Color::Black)` (or similar high-contrast reverse) to the selected region.

```rust
/// Apply selection highlight to a message's rendered lines.
/// Returns new Lines with Spans split and styled at selection boundaries.
fn apply_selection_highlight(
    lines: &[Line<'static>],
    start: TextPosition,
    end: TextPosition,
) -> Vec<Line<'static>> { ... }
```

**Color choice:** Reverse video (`bg(White).fg(Black)`) works universally against all existing foreground colors (Cyan user, White assistant, Green code, Yellow headings, DarkGray thinking, Red system).

#### Ctrl+C Dual Behavior

Modify the Ctrl+C handler in `input.rs:25-29`:

```rust
if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
    match &app.selection_state {
        SelectionState::Selected { .. } => {
            copy_selection_to_clipboard(app);
            app.selection_state = SelectionState::None; // clear after copy
        }
        SelectionState::Dragging { .. } => {
            app.selection_state = SelectionState::None; // cancel drag
        }
        SelectionState::None => {
            app.should_quit = true;
        }
    }
    return;
}
```

**Key behavior:** Ctrl+C copies AND clears selection, so a second Ctrl+C quits. No ambiguity.

#### Text Extraction for Copy

Extract plaintext from the rendered `Span` content (what the user sees, with markdown markers stripped). Walk the Spans within the selected range and concatenate their `content` strings. Include the role prefix ("You:", agent name) if it falls within the selection — the prefix is a visible Span and users expect to copy what they see.

Clipboard write via `arboard::Clipboard::new()?.set_text(text)`. On failure, display a system message (same pattern as image paste errors in `input.rs:233-240`).

### Implementation Phases

#### Phase 1: Per-Message Widget Refactor

**Goal:** Replace monolithic Paragraph with per-message rendering. No selection yet. All existing behavior preserved.

**Files changed:**

| File | Changes |
|------|---------|
| `tui/app.rs` | Add `MessageLayout`, `MessagesLayout` structs. Add `messages_layout: MessagesLayout` field to `App`. Add `messages_inner_rect: Option<Rect>` to store the messages area rect. |
| `tui/ui.rs` | Rewrite `draw_messages` to: (1) compute `MessagesLayout` if stale, (2) determine visible message range from scroll offset, (3) render each visible message as its own `Paragraph` in a sub-rect, (4) handle partial clipping for top/bottom edge messages, (5) store `screen_rect` on each visible `MessageLayout`. |
| `tui/ui.rs` | Extract `build_message_lines(msg, agent_name, is_last) -> Vec<Line<'static>>` helper from the current inline logic (lines 200-315). |

**Scroll model preserved:**

```
total_lines = sum of all message wrapped_lines + spacer lines
max_scroll = total_lines.saturating_sub(visible_height)
effective_scroll = max_scroll.saturating_sub(scroll_offset)  // inverted model

// Find first visible message via cumulative line offsets
// Render visible messages into sub-rects
// First visible message may be partially scrolled (use Paragraph::scroll)
```

**Invariants to verify:**
- `scroll_offset == 0` shows the latest message at the bottom
- `auto_scroll_to_bottom()` behavior unchanged
- `has_new_message` flag and footer indicator work
- Progressive reveal (streaming) renders correctly
- Thinking indicator appears/disappears correctly
- Team dashboard split-pane layout works
- Blank spacer lines between messages preserved

**Tests:**
- Unit test `MessagesLayout` computation with known messages and widths
- Unit test visible range calculation for various scroll offsets
- Verify `total_lines` matches the old `paragraph.line_count()` for the same content

#### Phase 2: Selection and Copy

**Goal:** Add mouse selection, visual highlight, and Ctrl+C copy.

**Files changed:**

| File | Changes |
|------|---------|
| `tui/app.rs` | Add `SelectionState` enum, `TextPosition` struct. Add `selection_state: SelectionState` field to `App`. |
| `tui/input.rs` | Expand `handle_mouse` to process `MouseDown`, `MouseDrag`, `MouseUp`. Modify `handle_key` Ctrl+C to be conditional on selection state. |
| `tui/ui.rs` | Add `hit_test` function. Add `apply_selection_highlight` function. Call highlight in `draw_messages` for the selected message. |
| `tui/ui.rs` | Update footer to show "Ctrl+C copy" when selection is active vs "Ctrl+C quit" when not. |

**Selection invalidation points (clear to `None`):**
- Scroll (mouse wheel, Ctrl+Up/Down, PageUp/PageDown) — in `scroll_up`/`scroll_down` on `App`
- Terminal resize — in resize event handler in `chat.rs`
- New message arrival — in `add_message` on `App`
- Content streaming — in `pending_response` update path
- Agent status change — in status transition logic
- Click without drag — in `MouseUp` handler when position equals anchor

**Streaming content:** Selection on the `pending_response` message is allowed but invalidated on any content change (next `pending_response` update). This means selections on streaming content are transient — the user can only successfully copy after streaming completes.

**Tests:**
- Unit test `hit_test` with known layouts: single-line message, wrapped message, CJK characters, empty message
- Unit test `apply_selection_highlight` with multi-Span Lines and various selection ranges
- Unit test text extraction from Spans
- Unit test selection state machine transitions
- Unit test Ctrl+C dual behavior (copy when selected, quit when not)
- Unit test clipboard write failure produces system message

## System-Wide Impact

### Interaction Graph

- `handle_mouse` (Down/Drag/Up) → updates `App.selection_state` → `draw_messages` reads selection state → applies highlight to affected message's Spans → renders modified Paragraph
- `handle_key` (Ctrl+C) → reads `App.selection_state` → if Selected: extracts text from Spans → `arboard::Clipboard::set_text()` → clears selection → redraw; if None: sets `should_quit`
- `scroll_up`/`scroll_down` → clears `App.selection_state` → redraw
- `add_message` / `set_pending_response` → invalidates `MessagesLayout` + clears selection

### Error Propagation

- Clipboard write failure: `arboard::Clipboard::new()` or `set_text()` returns `Err` → display system message ("Failed to copy to clipboard") → selection still cleared → no crash
- Hit-test outside messages area: returns `None` → no state change → no error

### State Lifecycle Risks

- **Stale `MessagesLayout`:** If layout is not recomputed when messages/width change, scroll will be wrong. Mitigation: invalidation flags checked at start of `draw_messages`.
- **`screen_rect` from previous frame:** Rect values are only valid for the frame they were computed in. Hit-test must use the current frame's rects. Mitigation: `screen_rect` is set during draw, hit-test runs against the same data in the same frame's event processing (events are processed before draw in the main loop — but rects are from the *previous* draw call). This requires storing rects after draw and using them for the *next* frame's events. Acceptable: one frame of latency is imperceptible.
- **Selection references a deleted message:** Messages are never deleted in the current TUI (only appended). No risk.

### API Surface Parity

- No API changes. This is CLI-only, no server/gateway impact.
- Team mode: selection works in the 70% messages panel. Dashboard panel (30%) does not support selection.

## Acceptance Criteria

### Functional Requirements

- [x] Per-message widget rendering produces identical visual output to the current monolithic Paragraph
- [x] Scroll behavior (inverted offset, auto-scroll, new-message indicator) works identically after refactor
- [x] Click-and-drag within a message highlights selected text with reverse-video styling
- [x] Dragging in both directions (forward and backward) works correctly
- [x] Drag is clamped to the originating message's bounds (no cross-message selection)
- [x] Mouse click without drag clears any existing selection
- [x] Ctrl+C with active selection copies plaintext to system clipboard and clears selection
- [x] Ctrl+C without selection exits the app (unchanged behavior)
- [x] Scroll clears active selection
- [x] Terminal resize clears active selection
- [x] Content updates (new message, streaming) clear active selection
- [x] Clipboard write failure shows a system message
- [x] Footer hint updates dynamically: "Ctrl+C copy" when selection active, "Ctrl+C quit" when not
- [x] Selection works in team mode split-pane layout (messages panel only)
- [x] Unicode content (CJK, emoji) selects and copies correctly

### Non-Functional Requirements

- [ ] No perceptible frame drops on conversations with 100+ messages
- [ ] Per-message rendering adds < 1ms overhead vs monolithic Paragraph for typical conversations

### Quality Gates

- [x] All existing TUI tests pass after Phase 1 refactor
- [x] New unit tests for: `MessagesLayout`, `hit_test`, `apply_selection_highlight`, text extraction, selection state machine, Ctrl+C dual behavior
- [x] `cargo clippy` clean
- [ ] Manual testing: basic selection, scroll clears, resize clears, streaming clears, copy works, quit works

## Dependencies & Prerequisites

- **ratatui 0.29** with `unstable-rendered-line-info` feature — already in `Cargo.toml`
- **arboard 3** — already in `Cargo.toml`, currently used for image paste read
- **unicode-width** — already in `Cargo.toml`
- **crossterm 0.28** — already captures all mouse events via `EnableMouseCapture`
- No new dependencies required

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Per-message scroll regression | Medium | High | Phase 1 is scroll refactor only — verify all scroll invariants before proceeding to Phase 2 |
| Hit-test inaccuracy (wrapping mismatch) | Medium | Medium | Unit test with known widths and content. Use `unicode_width` to match ratatui's internal width calculation |
| Performance regression on long conversations | Low | Medium | Only render visible messages. Cache `MessagesLayout` and invalidate incrementally |
| Clipboard write fails silently | Low | Low | Display system message on failure, matching existing image paste error pattern |
| Loss of terminal native selection | Low | High | Document that `EnableMouseCapture` suppresses terminal selection. Consider Shift+click passthrough as a future enhancement |

## Out of Scope

- Keyboard-based selection (Shift+Arrow) in input panel
- Cross-message selection
- Double-click word selection / triple-click line selection (future enhancement)
- Right-click context menu
- Search/find in conversation
- Message-level action buttons (future enhancement unlocked by per-message refactor)
- Selection in team dashboard panel

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-09-tui-text-selection-brainstorm.md](docs/brainstorms/2026-03-09-tui-text-selection-brainstorm.md) — Key decisions: mouse-only interaction, single-message selection, per-message widget refactor, Ctrl+C dual behavior

### Internal References

- Message rendering: `crates/mika-cli/src/tui/ui.rs:190-330` (draw_messages)
- Mouse handler: `crates/mika-cli/src/tui/input.rs:11-21` (handle_mouse)
- Ctrl+C handler: `crates/mika-cli/src/tui/input.rs:25-29`
- App state: `crates/mika-cli/src/tui/app.rs:264` (App struct)
- Scroll methods: `crates/mika-cli/src/tui/app.rs:919-940`
- Markdown renderer: `crates/mika-cli/src/tui/markdown.rs`
- Input wrapping: `crates/mika-cli/src/tui/ui.rs:19-38` (visual_line_rows — reference for unicode width walking)
- Clipboard image paste: `crates/mika-cli/src/tui/input.rs:200-240` (arboard usage pattern)
- Past solutions: `docs/solutions/ui-bugs/tui-persistent-history-and-paste-cursor.md` (text manipulation patterns), `docs/solutions/ui-bugs/tui-shell-like-autocompletion.md` (state machine pattern for modal UI)

### Related Work

- Issue: https://github.com/senara-solutions/mika/issues/72
