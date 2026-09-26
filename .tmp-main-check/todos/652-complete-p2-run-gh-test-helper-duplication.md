---
status: pending
priority: p2
issue_id: "652"
tags: [code-review, quality, architecture]
dependencies: []
---

# `run_gh`: Test helper `build_run_gh_command` duplicates production logic

## Problem Statement

The `build_run_gh_command` test helper (~47 lines) reimplements the entire validation pipeline from `run_gh` — string rejection, array parsing, empty check, allowlist check, repo appending, env construction. This creates a maintenance trap: if `run_gh` logic changes, the helper can silently drift out of sync. Tests would pass against the helper while the production code has bugs.

The `test_run_gh_env_scrubbing` test is particularly problematic — it tests the helper's `Vec<String>` collection, not the actual `cmd.env_remove()` calls in production code.

## Findings

- **Pattern recognition**: Noted this as a deviation from existing handler test patterns (no other handler has a parallel test helper).
- **Code simplicity**: Identified as the most significant complexity issue, recommended deletion.
- **Architecture**: Recommended extracting shared validation function.
- **Performance**: Echoed the same concern about maintenance liability.

## Proposed Solutions

### Solution 1: Extract shared validation function (Recommended)
Extract the validation and argument-building logic from `run_gh` into a pure function:
```rust
struct GhArgs {
    args: Vec<String>,
    repo: Option<String>,
}

fn validate_gh_input(input: &serde_json::Value) -> Result<GhArgs, String> { ... }
```
Then `run_gh` calls this function and spawns the process; tests call it directly. Delete `build_run_gh_command` entirely.
- **Pros**: Single source of truth, tests validate real logic, cleaner architecture
- **Cons**: Minor refactor
- **Effort**: Small
- **Risk**: Low

### Solution 2: Delete helper, test only via `run_gh` directly
Remove `build_run_gh_command` and all tests that use it. The allowlist, repo appending, and env scrubbing are already implicitly tested through `run_gh` calls or are untestable without process spawning.
- **Pros**: Simplest approach, removes ~65 lines
- **Cons**: Loses granular arg-construction tests
- **Effort**: Small
- **Risk**: Low

## Recommended Action

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/builtin_handlers.rs` (lines 346-393 for helper, lines 603-668 for dependent tests)
- **Components**: `run_gh` builtin handler tests

## Acceptance Criteria

- [ ] `build_run_gh_command` is deleted
- [ ] Validation logic is shared between production code and tests (if Solution 1)
- [ ] No duplicated validation logic between production and test code
- [ ] All existing test coverage is preserved or improved

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Unanimous finding across 4 reviewers |

## Resources

- Existing todo 045 (completed): Previous test helper duplication issue
