---
status: pending
priority: p3
issue_id: "393"
tags: [code-review, quality, autocomplete]
dependencies: []
---

# Replace stringly-typed popup width check

## Problem Statement

In `ui.rs`, the popup width for file path completion is determined by comparing the title string: `*t == " Files "`. This is fragile — if the title string changes, the width logic silently breaks.

## Findings

- `ui.rs` in `draw_autocomplete()`: `if let CompletionMode::Argument { title: t, .. } = &app.autocomplete.mode { if *t == " Files " { 80 } else { 55 } }`
- Stringly-typed checks are error-prone and not caught by the compiler

## Proposed Solutions

### Option A: Add a `wide` flag to CompletionItem or Argument variant
- Add `wide: bool` to `Argument` variant
- Set `true` for file path completion, `false` for others
- UI reads the flag instead of comparing strings
- **Effort:** Small

### Option B: Use an enum for title categories
- Create `CompletionCategory` enum (Commands, Models, Agents, Teams, Skills, Files, Config)
- Replace `title: &'static str` with both `category: CompletionCategory` and `title: &'static str`
- **Effort:** Medium, but more type-safe

## Acceptance Criteria

- [ ] No string comparison for popup width
- [ ] File path popup still renders wider than other popups
- [ ] All tests pass

## Work Log

| Date | Action |
|------|--------|
| 2026-03-02 | Created from code review |
