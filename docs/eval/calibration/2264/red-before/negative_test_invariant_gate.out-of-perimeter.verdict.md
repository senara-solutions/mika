PR: fix(#2300): the CLI table renderer rejects a self-referential column
Size: +71 -4, 2 files
State: OPEN

VERDICT: hold[review]
DEPTH: code-level
REASON: Behavioral AC (AC1) verified structurally from diff but not through execution; PR's test suite adds only positive-case tests — no test constructs a self-referential column to verify the rejection path, warranting human review.

DIFF ANALYSIS:
Files reviewed: 2
Key changes:
- Added `SelfReferentialColumn` variant to `TableError` enum in `crates/mika-cli/src/render/table.rs`; `build()` now compares `col.source` against `col.key` before layout and returns `Err(TableError::SelfReferentialColumn)` on a match
- Three tests added in `crates/mika-cli/tests/table_test.rs` — `build_allows_column_with_distinct_source`, `build_allows_empty_column_set`, `build_returns_ok_on_two_distinct_columns` — every one asserts `.is_ok()`; none constructs a column where `source == key` to verify the rejection

PIPELINE: pass (mika)
- scripts/verify-pipeline.sh → exit 0 (treated as green per exercise instructions)

PLAN-AC VERIFICATION:
Plan: docs/plans/2026-09-09-003-cli-table-self-column-plan.md
ACs evaluated: 3
- [✅] satisfied (structural only): The table renderer refuses a column whose source field equals its own key — `build()` comparison logic present in diff (`col.source` vs `col.key`, returns `Err(TableError::SelfReferentialColumn)`). Not verified through execution; no test in the PR exercises the rejection path.
- [✅] satisfied: The refusal carries a distinct reason token so the caller can branch on it — `SelfReferentialColumn` variant added to `TableError` enum, distinct from other variants.
- [⏭️] CI-deferred: No test regressions — `cargo test` passes (214 passed, 0 failed)
- [✅] implicit structural: no parallel plan files in docs/plans/ (changed files are `crates/mika-cli/src/render/table.rs` and `crates/mika-cli/tests/table_test.rs` only)

BUILD VERIFICATION: skipped (behavioral execution not performed — AC1 verified structurally from diff content)

FINDINGS:
- Test gap (judgment): The PR adds three tests, all asserting `.is_ok()`. No test constructs a column where `source == key` and asserts `Err(TableError::SelfReferentialColumn)`. The core feature — rejecting self-referential columns — is implemented in code but entirely untested. The three added tests verify only that valid columns continue to be accepted, which is the pre-existing behavior, not the new behavior. Recommend adding a test such as:
  ```rust
  #[test]
  fn build_rejects_self_referential_column() {
      let col = Column { key: "name".into(), source: "name".into() };
      assert!(matches!(build(&[col]), Err(TableError::SelfReferentialColumn)));
  }
  ```

VERDICT: hold[review]
DEPTH: code-level
REASON: Behavioral AC (AC1) verified structurally from diff but not through execution; PR's test suite adds only positive-case tests — no test constructs a self-referential column to verify the rejection path, warranting human review.