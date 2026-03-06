---
status: complete
priority: p3
issue_id: "510"
tags: [code-review, consistency, maintainability]
dependencies: []
---

# `trigger_type` Constants Defined but Unused at Call Sites — Bare String Literals Everywhere

## Problem Statement

`trigger_type` constants were added to `types.rs` in this PR but zero call sites use them. Every comparison and assignment uses bare string literals (`"callback"`, `"time"`, `"recurring"`). The `action_type` constants in the same file are used correctly. This is an internal inconsistency within the PR itself.

## Findings

- **Source**: architecture-strategist (F-6 Low), patterns-reviewer (F-1 Minor)
- **Location**: Multiple files

Bare string literal sites:
- `handlers.rs:352` — `"callback"` comparison
- `engine.rs:254` — `"recurring"` string comparison
- `engine.rs:365` — comment reference
- `mod.rs:33` — `"recurring".to_string()`
- `create_task.rs:94,127,151,170` — match arm literals for `"time"`, `"recurring"`, `"callback"`
- `list_tasks.rs` — `"time"` references
- `list_reminders.rs`, `cancel_reminder.rs`, `queue.rs` test helper

The `trigger_type` module exists in `types.rs` lines 27–35 and is re-exported from `mod.rs`. It is simply not imported at call sites.

## Proposed Solutions

### Option A: Replace all bare literals with constants (Recommended)

Import `use crate::task_engine::trigger_type;` in each file and replace:
- `"callback"` → `trigger_type::CALLBACK`
- `"time"` → `trigger_type::TIME`
- `"recurring"` → `trigger_type::RECURRING`

This is a mechanical find-and-replace with no behavior change.

- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] No bare `"callback"`, `"time"`, or `"recurring"` string literals used in trigger_type comparisons/assignments
- [ ] All call sites use `trigger_type::CALLBACK`, `trigger_type::TIME`, `trigger_type::RECURRING` constants
- [ ] `cargo test` passes unchanged

## Work Log

- 2026-03-06: Identified by architecture-strategist and patterns-reviewer of feat/unified-task-engine
