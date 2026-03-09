# Brainstorm: TUI Text Selection (Issue #72)

**Date:** 2026-03-09
**Status:** Draft
**Issue:** https://github.com/senara-solutions/mika/issues/72

## What We're Building

Mouse-based text selection and copy in the TUI conversation panel. Users can click-and-drag to highlight text within a single message, then Ctrl+C to copy it to the system clipboard.

## Why This Approach

### Interaction Model: Mouse-only

- Click-and-drag to select text in the conversation panel
- Ctrl+C copies selected text when a selection exists; exits the app when nothing is selected (preserving current behavior)
- Visual highlight overlay on selected text
- No keyboard-based selection (Shift+Arrow) in the input panel — out of scope

### Selection Scope: Single message

- Selection is constrained to one message at a time
- Simpler coordinate mapping and avoids complex cross-message boundary logic
- Sufficient for the primary use case (copying a code snippet or response excerpt)

### Rendering: Per-message widgets (refactor)

- **Refactor `draw_messages` from a single `Paragraph` to per-message widget rendering**
- Each message rendered as its own `Paragraph` in a manual vertical layout
- Per-message coordinate boundaries come for free — no shadow data structures
- Click target identification: "which widget am I in, what's the offset within it"
- Scroll logic reworked to operate on per-message heights rather than a global line count
- This approach unlocks future features (click-to-investigate, message actions, expand/collapse tool results) without another refactoring pass

### Copy Mechanism

- `arboard` already a dependency (used for image paste via Ctrl+V)
- On Ctrl+C with active selection: write plain text to clipboard, clear selection
- On Ctrl+C without selection: exit app (current behavior preserved)

## Key Decisions

1. **Mouse-only interaction** — no keyboard selection, no vi-mode
2. **Single-message selection** — no cross-message drag
3. **Per-message widget refactor** — break the monolithic `Paragraph` into individual message widgets with known screen coordinates
4. **Ctrl+C dual behavior** — copies when selection exists, exits when it doesn't
5. **Selection clears on scroll or click elsewhere** — standard UX

## Technical Considerations

- **Scroll refactor:** Current scroll uses inverted offset model against a single Paragraph's `line_count()`. Per-message rendering needs cumulative height tracking per message, with scroll offset applied to the layout.
- **Progressive reveal:** During streaming responses, the active message content changes every tick. Selection on the streaming message should either be disallowed or cleared on content change.
- **Markdown rendering:** The existing `markdown.rs` produces `Vec<Line<'static>>` with styled Spans. Selection highlighting needs to overlay (e.g., reverse video) on top of existing styles.
- **Mouse event handling:** `handle_mouse` in `input.rs` currently ignores click/drag/release. Need to handle `MouseEventKind::Down`, `Drag`, `Up` for selection state machine.
- **Coordinate mapping:** Need to map screen (column, row) to (message_index, character_offset) using the per-message layout rectangles and ratatui's wrapping.

## Out of Scope

- Keyboard-based selection in the input panel
- Cross-message selection
- Right-click context menu
- Search/find in conversation
- Message-level action buttons (future feature unlocked by the refactor, but not built now)
