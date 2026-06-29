# PR Review: fix(api): ensure no null values in JSON response fields

## PR Metadata
- **Title:** fix(api): ensure no null values in JSON response fields
- **State:** OPEN
- **Draft:** false
- **Base:** main
- **Head:** fix/1180/null-free-json
- **Files changed:** 2
- **Additions:** 35
- **Deletions:** 8

## Plan Path
`docs/plans/1180-null-free-json-plan.md`

## Acceptance Criteria
- AC1: All JSON response fields use empty string `""` instead of `null` for optional string fields
- AC2: `TaskResponse` serialization never produces `null` for `source`, `reference_url`, or `metadata` fields
- AC3: No `null` values appear in any JSON response field across the task API endpoints

## Diff Summary

### `crates/mika-agent/src/server/handlers.rs`
- `TaskResponse` now uses `#[serde(serialize_with = "serialize_option_string")]` on `source`, `reference_url`, `metadata`
- Added `serialize_option_string` helper: `None` serializes as `""`, `Some(v)` serializes as `v`

### `crates/mika-agent/src/server/handlers.rs` (tests)
- Added `test_task_response_no_nulls` — creates a TaskResponse with None fields, serializes to JSON, asserts no `null` token in output string
- Added `test_task_response_with_values` — creates a TaskResponse with Some fields, verifies values preserved

## DIFF ANALYSIS
- AC1: `serialize_option_string` converts `None` to `""` — implementation present
- AC2: Three fields annotated with custom serializer — implementation present
- AC3: This AC asserts absence of null values across ALL task API endpoints. The diff only modifies `TaskResponse`. Other response types (`TaskDetailResponse`, `TaskChildrenResponse`) are not modified.
