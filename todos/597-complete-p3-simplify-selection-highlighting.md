---
status: complete
priority: p3
issue_id: "597"
tags: [code-review, simplicity, tui]
dependencies: ["596"]
---

# Simplify textarea selection highlighting via post-processing

## Problem Statement

`build_selection_line()` (~40 lines) duplicates the span-splitting logic already in `apply_selection_highlight()`. The textarea selection is rendered by building per-character `Vec<bool>` flags during the wrapping loop and grouping them into spans, when it could instead be applied as a post-processing step on already-wrapped lines (same as message selection).

## Findings

- Code Simplicity Reviewer identified `build_selection_line` as redundant with `apply_selection_highlight`
- The per-char bool flag approach forces a different grouping algorithm and adds ~60 lines of selection-related code to `wrap_input_with_cursor`
- `apply_selection_highlight` already handles the same job for message-area selection using `TextPosition` coordinates

## Proposed Solutions

### Option A: Post-process wrapped lines with `apply_selection_highlight`
After `wrap_input_with_cursor` produces plain wrapped lines, convert the textarea selection range `((start_row, start_col), (end_row, end_col))` into display-line coordinates and apply `apply_selection_highlight`. Remove `build_selection_line`, `sel_flags`, and all selection branching from the wrapping loop.

- **Pros:** Eliminates ~75 lines, single code path for all selection highlighting
- **Cons:** Requires coordinate mapping from logical to display-line space (already tracked via cursor_y)
- **Effort:** Medium
- **Risk:** Low

### Option B: Keep separate but extract shared span-splitting
Extract the core span-splitting logic into a shared helper used by both functions.

- **Pros:** Less invasive, preserves current approach
- **Cons:** Still two call sites, partial dedup
- **Effort:** Small
- **Risk:** Low

## Technical Details

**Affected files:**
- `crates/mika-cli/src/tui/ui.rs` — `build_selection_line`, `wrap_input_with_cursor`, `apply_selection_highlight`

## Acceptance Criteria

- [ ] `build_selection_line` removed or merged with `apply_selection_highlight`
- [ ] Textarea selection highlighting still renders correctly
- [ ] All tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-09 | Created from code review | Dependent on wrapping dedup (596) |
