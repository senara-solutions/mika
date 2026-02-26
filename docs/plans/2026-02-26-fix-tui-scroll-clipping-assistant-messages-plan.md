---
title: "fix: TUI scroll clipping hides assistant message content"
type: fix
status: completed
date: 2026-02-26
---

# fix: TUI scroll clipping hides assistant message content

## Problem Statement

When Mika responds in the TUI, the "Mika:" label renders correctly but the actual message content below it is invisible. The message IS saved to the database correctly (confirmed via DataGrip), and `mika ask` prints it to stdout. The TUI just doesn't display it.

**Reproduction:** Run `mika ask "hello"` from a separate terminal while the TUI is open. The TUI picks up both the user message and the assistant response via cross-channel polling, but only shows "Mika:" with no content below it.

**Evidence from screenshots:**
- DataGrip shows row 35: role=assistant, content="Hey! Still here. What's up?"
- Terminal shows agent completed with that same text
- TUI shows "Mika:" on its own line, then nothing — input prompt follows immediately

## Root Cause Analysis

The bug is in `draw_messages()` in `crates/mika-cli/src/tui/ui.rs` (lines 202-228). The scroll offset calculation manually estimates how many visual rows the content occupies after word-wrapping:

```rust
// ui.rs:210-221 — CURRENT (buggy) estimation
lines.iter().map(|line| {
    let w = line.width();
    if w == 0 { 1 } else { (w.saturating_sub(1) / viewport_width) + 1 }
}).sum()
```

This uses a **character-count-based** estimate: `ceil(line_width / viewport_width)`. But ratatui's `Paragraph` widget with `Wrap { trim: false }` uses **word-boundary wrapping** (`WordWrapper`), which can produce MORE visual rows than this estimate when a word straddles a line boundary.

Over a long conversation with many wrapped lines, the cumulative undercount grows. Eventually `max_scroll` (= `total_lines - visible_height`) is too small, the `Paragraph::scroll()` offset doesn't reach the true bottom, and the last few lines of content are clipped below the viewport.

**Why "Mika:" shows but content doesn't:** The label line is short (fits in one row, no wrap discrepancy). It falls just within the visible area. The content lines after it are the very bottom of the conversation — they fall 1-2 rows below the viewport due to the accumulated scroll error.

## Proposed Solution

Replace the manual wrap estimation with `Paragraph::line_count(width)` — a method added in ratatui 0.28 that returns the **exact** number of visual rows the paragraph occupies after wrapping. This eliminates the estimation discrepancy entirely.

### Implementation

**File: `crates/mika-cli/src/tui/ui.rs` — `draw_messages()` function**

Replace lines 202-232 (the scroll calculation + paragraph creation) with:

```rust
// Build paragraph with wrapping first (no scroll yet)
let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });

// Use ratatui's own line counting for accurate scroll (accounts for word wrapping)
let total_lines = paragraph.line_count(inner.width) as usize;
let visible_height = inner.height as usize;
let max_scroll = total_lines.saturating_sub(visible_height);
let effective_scroll = max_scroll.saturating_sub(app.scroll_offset);

let scroll_u16 = effective_scroll.min(u16::MAX as usize) as u16;
let paragraph = paragraph.scroll((scroll_u16, 0));
f.render_widget(paragraph, inner);
```

This is a ~15-line replacement in a single function. No other files need changes.

## Acceptance Criteria

- [x] Assistant message content renders below "Mika:" label in the TUI
- [x] Messages from `mika ask` (cross-channel polled) display correctly
- [x] Messages typed directly in TUI still render correctly (progressive reveal + committed)
- [x] Scroll up/down (PageUp/PageDown) still works
- [x] Long conversations with many wrapped lines don't clip the bottom
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## MVP

### crates/mika-cli/src/tui/ui.rs

Replace the manual scroll estimation block (lines ~202-232) with `Paragraph::line_count()`:

```rust
// Build paragraph with wrapping first, then use ratatui's accurate line counting
let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
let total_lines = paragraph.line_count(inner.width) as usize;
let visible_height = inner.height as usize;
let max_scroll = total_lines.saturating_sub(visible_height);
let effective_scroll = max_scroll.saturating_sub(app.scroll_offset);
let scroll_u16 = effective_scroll.min(u16::MAX as usize) as u16;
let paragraph = paragraph.scroll((scroll_u16, 0));
f.render_widget(paragraph, inner);
```

## References

- ratatui 0.28 release: added `Paragraph::line_count(width)` for accurate wrapped line counting
- Current ratatui version: 0.29.0 (confirmed in Cargo.lock)
- Related prior fix: `docs/solutions/ui-bugs/tui-ask-visibility-skill-seeder-config-tools.md`
- Related prior fix: `docs/solutions/ui-bugs/tui-log-corruption-and-empty-agent-replies.md`
