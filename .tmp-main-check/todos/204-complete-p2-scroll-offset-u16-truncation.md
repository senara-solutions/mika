---
status: complete
priority: p2
issue_id: "204"
tags: [code-review, correctness, tui]
dependencies: []
---

# Scroll Offset Uses u16 — Truncates at 65,535 Lines

## Problem Statement

`scroll_offset` is `u16` and `total_lines` is cast via `lines.len() as u16`. Long conversations with markdown-heavy responses can exceed 65,535 rendered lines, causing silent truncation and broken scroll behavior.

## Findings

- **Source:** performance-oracle (Issue 3.3), architecture-strategist (9c)
- **Location:** `crates/mika-cli/src/tui/app.rs:42`, `crates/mika-cli/src/tui/ui.rs:121`
- **Evidence:** `let total_lines = lines.len() as u16;` — silent truncation above 65,535
- **Impact:** Scroll viewport jumps to wrong position in long sessions

## Proposed Solutions

### Option 1: Use usize internally, clamp to u16 at rendering boundary
- **Pros**: Correct arithmetic, only clamps at ratatui's `Paragraph::scroll()` call
- **Cons**: Minor type changes
- **Effort**: Small
- **Risk**: Low

## Recommended Action

Option 1.

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/app.rs`, `crates/mika-cli/src/tui/ui.rs`

## Acceptance Criteria

- [ ] Scroll works correctly with >65,535 rendered lines
- [ ] No silent truncation in scroll calculations

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | |

## Resources

- Commit: 399ebf0
