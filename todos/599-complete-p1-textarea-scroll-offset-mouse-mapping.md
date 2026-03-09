---
status: complete
priority: p1
issue_id: "599"
tags: [code-review, bug, tui]
dependencies: []
---

# Fix textarea scroll offset not accounted for in mouse coordinate mapping

## Problem Statement

`screen_to_textarea_pos()` did not account for the textarea's scroll offset when mapping screen coordinates to logical text positions. When the textarea content exceeds the visible height (e.g., multi-line input), the display scrolls to keep the cursor visible. Mouse clicks in a scrolled textarea would map to the wrong logical line — off by `scroll_offset` rows.

## Findings

- Architecture Strategist flagged this as a **critical bug** during code review
- `draw_input()` computes `scroll_offset` and skips that many display rows when rendering
- `screen_to_textarea_pos()` treated `rel_row` as absolute display row 0-indexed, without adding the scroll offset
- The textarea is capped at 6 display lines, so the bug manifests when input wraps to 7+ lines

## Resolution

- Added `textarea_scroll_offset: u16` field to `App` struct
- Store the computed scroll offset during `draw_input()`
- Add scroll offset to `rel_row` in `screen_to_textarea_pos()` before walking display lines

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-09 | Found and fixed during code review | Always sync coordinate mapping with render-time scroll state |
