---
status: complete
priority: p3
issue_id: "593"
tags: [code-review, quality, tui]
dependencies: []
---

# Derive Ord on TextPosition

## Problem Statement

`TextPosition` uses a manual `is_before_or_equal` method for comparison. Since `TextPosition` has a natural ordering (line first, then char_offset), it should derive `Ord`/`PartialOrd` to use standard comparison operators (`<=`, `<`, etc.).

## Findings

- `crates/mika-cli/src/tui/ui.rs` — `is_before_or_equal` method and call sites
- `TextPosition { line: usize, char_offset: usize }` in `app.rs`
- Standard `Ord` derive would sort by line first, then char_offset — matching the manual implementation

## Proposed Solutions

### Solution 1: Derive Ord (Recommended)
Add `#[derive(PartialOrd, Ord)]` to `TextPosition` and replace `is_before_or_equal(a, b)` with `a <= b`.

**Pros:** Idiomatic Rust, removes custom comparison function
**Cons:** None
**Effort:** Small
**Risk:** Low

## Acceptance Criteria

- [ ] `TextPosition` derives `PartialOrd` and `Ord`
- [ ] `is_before_or_equal` removed
- [ ] All comparison sites use `<=` or `<`
- [ ] Tests pass

## Work Log

| Date | Action |
|------|--------|
| 2026-03-09 | Created from PR #93 code review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/93
