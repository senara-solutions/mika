---
status: complete
priority: p3
issue_id: "392"
tags: [code-review, quality, autocomplete]
dependencies: []
---

# Remove dead command_name field from Argument variant

## Problem Statement

The `CompletionMode::Argument` variant carries a `command_name: &'static str` field that is marked `#[allow(dead_code)]`. It was added for potential future use (e.g., contextual behavior based on which command is active) but is currently unused outside of the `show_arguments()` call that sets it.

## Findings

- `autocomplete.rs:36-37`: `#[allow(dead_code)] command_name: &'static str`
- Only set in `show_arguments()`, never read in production code
- `trigger_argument_completion` in `input.rs` already knows the command name from parsing

## Proposed Solutions

### Option A: Remove the field
- Remove `command_name` from Argument variant
- Remove `command_name` parameter from `show_arguments()`
- **Pros:** Less YAGNI, cleaner code
- **Cons:** Would need to re-add if ever needed
- **Effort:** Small

## Acceptance Criteria

- [x] `command_name` field removed from Argument variant
- [x] `show_arguments()` signature updated
- [x] All tests pass

## Work Log

| Date | Action |
|------|--------|
| 2026-03-02 | Created from code review |
| 2026-03-02 | Completed: removed command_name field, updated show_arguments() signature and all callers |
