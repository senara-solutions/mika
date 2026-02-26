---
title: TUI scroll clipping hides assistant message content due to inaccurate line counting
date: 2026-02-26
module: mika-cli/tui
severity: high
tags: [rendering, scrolling, ratatui, word-wrap, ui]
symptoms:
  - Assistant message content clipped below "Mika:" label
  - Clipping worse over longer conversations
  - Cross-channel polled messages (from mika ask) show label but not content
root_cause_category: estimation-algorithm-mismatch
---

# TUI scroll clipping hides assistant message content

## Problem

When Mika responds in the TUI, the "Mika:" label renders correctly but the actual message content below it is invisible. The message IS saved to the database correctly (confirmed via DataGrip) and `mika ask` prints it to stdout. The TUI just doesn't display it.

**Symptoms:**
- "Mika:" label visible, content missing
- Bug is worse on longer conversations (accumulating error)
- Affects both cross-channel polled messages and direct TUI responses
- Status bar shows "ready" (agent completed normally)

## Root Cause

The bug is in `draw_messages()` in `crates/mika-cli/src/tui/ui.rs`. The scroll offset calculation manually estimated how many visual rows the content would occupy after word-wrapping:

```rust
// BUGGY: character-count-based estimation
let total_lines: usize = lines.iter().map(|line| {
    let w = line.width();
    if w == 0 { 1 } else { (w.saturating_sub(1) / viewport_width) + 1 }
}).sum();
```

This formula assumes lines wrap at exact character boundaries. But ratatui's `Paragraph` widget with `Wrap { trim: false }` uses **word-boundary wrapping** via `WordWrapper`, which can produce MORE visual rows when a word straddles a line boundary.

Over a long conversation with many wrapped lines, the cumulative undercount grows. Eventually `max_scroll` (= `total_lines - visible_height`) is too small, `Paragraph::scroll()` doesn't reach the true bottom, and the last few lines of content are clipped below the viewport.

**Why "Mika:" shows but content doesn't:** The label line is short (fits in one row, no wrap discrepancy). It falls just within the visible area. The content lines immediately after it are the very bottom of the conversation and fall 1-2 rows below the viewport due to the accumulated scroll error.

## Solution

Replace the manual wrap estimation with `Paragraph::line_count(width)` -- ratatui's built-in method that returns the **exact** number of visual rows after wrapping.

**1. Enable the feature flag in `Cargo.toml`:**

```toml
# unstable-rendered-line-info: enables Paragraph::line_count() for accurate scroll (stable in practice since 0.25)
ratatui = { version = "0.29", features = ["unstable-rendered-line-info"] }
```

**2. Replace scroll estimation in `crates/mika-cli/src/tui/ui.rs`:**

```rust
// Build paragraph with wrapping first, then use ratatui's accurate line counting
// to calculate scroll. This avoids the discrepancy between our manual character-count
// estimation and ratatui's word-boundary wrapping (WordWrapper), which can produce
// more visual rows when words straddle line boundaries.
let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
let total_lines = paragraph.line_count(inner.width);
let visible_height = inner.height as usize;
let max_scroll = total_lines.saturating_sub(visible_height);
let effective_scroll = max_scroll.saturating_sub(app.scroll_offset);

let scroll_u16 = effective_scroll.min(u16::MAX as usize) as u16;
let paragraph = paragraph.scroll((scroll_u16, 0));
f.render_widget(paragraph, inner);
```

**Performance note:** `line_count()` runs the `WordWrapper` state machine once. The subsequent `render()` runs it again. At 500 messages (~40K graphemes), the additional pass costs ~0.1-0.5ms -- well within the 30ms frame budget. Conversation compaction at 50 messages further bounds growth.

## Prevention

### Don't reimplement library internals

When you need to coordinate with a library's rendering (scroll position, layout), source the ground truth from the library itself. Character-width math will inevitably drift from the library's actual wrapping algorithm.

```
Bad:  Calculate line count independently, hope it matches the widget
Good: Ask the widget for its line count, use that as source of truth
```

### Code review red flags

- Custom wrapping logic (`.chars().fold()` or manual width calculation) alongside `Paragraph::wrap()`
- Scroll clamping that uses a different width than the render area
- Comments containing "estimate" or "approximate" near scroll calculations

### Ratatui-specific scroll pattern

```rust
// Always: build paragraph first, then query its line_count
let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
let total_lines = paragraph.line_count(width);
let scroll = calculate_scroll(total_lines, visible_height, offset);
let paragraph = paragraph.scroll((scroll, 0));
f.render_widget(paragraph, area);
```

## Related

- [TUI log corruption and empty agent replies](../runtime-errors/tui-log-corruption-and-empty-agent-replies.md) -- previous TUI display bugs
- [TUI ask visibility and cross-channel polling](tui-ask-visibility-skill-seeder-config-tools.md) -- cross-channel polling mechanism
- [Multi-channel TUI visibility](multi-channel-tui-visibility-cross-channel-polling.md) -- watermark-based polling pattern
- [Agent skill hallucination and TUI scroll cutoff](../logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md) -- earlier scroll fix that introduced the character-count estimation
- ratatui tracking issue for `line_count()`: https://github.com/ratatui/ratatui/issues/293
