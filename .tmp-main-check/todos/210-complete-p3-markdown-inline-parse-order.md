---
status: complete
priority: p3
issue_id: "210"
tags: [code-review, correctness, tui]
dependencies: []
---

# Markdown Inline Code Before Bold Creates Incorrect Parsing

## Problem Statement

The `render_inline()` function checks for `**` before backtick. If text contains `` `code **not bold**` ``, the function finds `**` first and treats it as bold, misrendering inline code that contains asterisks.

## Findings

- **Source:** architecture-strategist (9d)
- **Location:** `crates/mika-cli/src/tui/markdown.rs:64-78`
- **Evidence:** Function checks `remaining.find("**")` before `remaining.find('`')`. The earlier marker should win regardless of type.
- **Impact:** Occasional misrendering of inline code containing asterisks in Claude responses

## Proposed Solutions

### Option 1: Find earliest marker of any type
- **Pros**: Correct parsing order
- **Cons**: Slightly more complex logic
- **Effort**: Small
- **Risk**: Low

Find the position of both `**` and `` ` ``, process whichever comes first.

## Recommended Action

Option 1.

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/markdown.rs`

## Acceptance Criteria

- [ ] `` `code **text**` `` renders as inline code, not bold inside code
- [ ] `**bold `code` bold**` renders bold with inline code inside

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | |

## Resources

- Commit: 399ebf0
