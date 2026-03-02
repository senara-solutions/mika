---
status: complete
priority: p3
issue_id: "391"
tags: [code-review, quality, autocomplete]
dependencies: []
---

# Remove CompletionItem duplication from Command mode

## Problem Statement

In `CompletionMode::Command`, each entry stores both `&'static SlashCommand` and a `CompletionItem`. The `CompletionItem` is constructed from `SlashCommand` data (name + description) but the UI rendering in `draw_autocomplete()` also reads from the `SlashCommand` directly. This creates unnecessary duplication.

## Findings

- `autocomplete.rs:30-33`: Command variant stores `Vec<(&'static SlashCommand, CompletionItem)>`
- `update_command()` constructs `CompletionItem` from `cmd.name` and `cmd.description`
- `ui.rs` in `draw_autocomplete` renders using the `CompletionItem` values
- The `SlashCommand` reference is used for `selected_command()` and `args_hint` checks
- Simplification: Command mode could store only `Vec<&'static SlashCommand>` and render directly from SlashCommand fields

## Proposed Solutions

### Option A: Remove CompletionItem from Command variant
- Change to `Command { items: Vec<&'static SlashCommand>, selected: usize }`
- Update `item_values()` to read from `cmd.name`
- Update `draw_autocomplete` to read description from `cmd.description` + `cmd.args_hint`
- **Pros:** Less allocation, single source of truth
- **Cons:** Slight code churn
- **Effort:** Small

## Acceptance Criteria

- [x] Command mode no longer constructs CompletionItem
- [x] All existing tests pass
- [x] UI rendering unchanged

## Work Log

| Date | Action |
|------|--------|
| 2026-03-02 | Created from code review |
| 2026-03-02 | Implemented Option A: removed CompletionItem from Command variant |
