# PR Review: feat(task-engine): add dispatch class filtering to list_tasks

## PR Metadata
- **Title:** feat(task-engine): add dispatch class filtering to list_tasks
- **State:** OPEN
- **Draft:** false
- **Base:** main
- **Head:** feat/1300/dispatch-class-filter
- **Files changed:** 5
- **Additions:** 112
- **Deletions:** 18

## Plan Path
`docs/plans/1300-dispatch-class-filter-plan.md`

## Acceptance Criteria
- AC1: `list_tasks` tool accepts optional `dispatch_class` filter parameter
- AC2: SQL query uses parameterized WHERE clause (no string interpolation)
- AC3: Dashboard TaskList component passes dispatch_class filter to API
- AC4: API endpoint `/api/v1/tasks` accepts `dispatch_class` query param
- AC5: Unit test covers filtering by both `implement` and `groom` classes

## Diff Summary

### `crates/mika-agent/src/tools/list_tasks.rs`
- Added `dispatch_class: Option<String>` to `ListTasksInput`
- Added validation: value must be "implement" or "groom" if provided
- Passes filter to `db.list_manual_tasks()`

### `crates/mika-agent/src/db.rs`
- `list_manual_tasks()` gains `dispatch_class: Option<&str>` parameter
- SQL uses `AND COALESCE(dispatch_class, 'implement') = ?1` when filter is Some
- Parameter bound via `rusqlite::params![]`

### `crates/mika-agent/src/server/handlers.rs`
- `GET /api/v1/tasks` extracts `dispatch_class` from query params
- Passes to `db.list_manual_tasks()`

### `dashboard/src/pages/TaskList.tsx`
- No changes to dashboard component

### `crates/mika-agent/src/db.rs` (tests)
- Added `test_list_tasks_filter_by_dispatch_class_implement`
- Tests filtering by `implement` class only — does NOT test `groom` class

## DIFF ANALYSIS
- AC1: `dispatch_class` parameter added to `ListTasksInput` — SATISFIED
- AC2: SQL uses `?1` parameter binding via `rusqlite::params![]` — SATISFIED
- AC3: Dashboard TaskList.tsx has NO changes — UNSATISFIED
- AC4: API endpoint extracts and passes `dispatch_class` — SATISFIED
- AC5: Test only covers `implement` class, not `groom` — UNSATISFIED
