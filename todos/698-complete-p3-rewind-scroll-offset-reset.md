---
status: complete
priority: p3
issue_id: 698
tags: [code-review, tui, rewind]
dependencies: []
---

# Reset scroll_offset after TUI rewind

## Problem Statement

After `/rewind N` in the TUI, `app.scroll_offset` is not reset to 0. The `/clear` command (line 91-95 in handlers.rs) resets scroll, but the rewind handler does not. After rewinding, the scroll position could point past the end of the now-shorter message list, causing rendering artifacts until the next scroll event.

## Findings

- Pattern recognition agent identified this inconsistency with `/clear` handler
- `MessagesLayout::default()` reset was correctly added but `scroll_offset` was missed
- Pre-existing in the cross-session path; now applies to both paths after unification

## Proposed Solutions

### Option 1: Add `app.scroll_offset = 0` after layout reset (Recommended)
- **Pros:** One line, matches `/clear` pattern, defensive
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Technical Details

- **File:** `crates/mika-cli/src/tui/commands/handlers.rs` ~line 962
- **Add after:** `app.messages_layout = MessagesLayout::default();`

## Acceptance Criteria

- [ ] `app.scroll_offset = 0` added after rewind message reload
- [ ] Manual test: scroll down in long conversation, `/rewind 1`, verify display starts at bottom (most recent)
