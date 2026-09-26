# PR Review: fix(#2300): the CLI table renderer rejects a self-referential column

## PR Metadata
- **Title:** fix(#2300): the CLI table renderer rejects a self-referential column
- **State:** OPEN
- **Draft:** false
- **Base:** main
- **Head:** fix/2300/cli-table-self-column
- **Files changed:** 2
- **Additions:** 71
- **Deletions:** 4
- **Latest commit:** `fix(#2300): reject a column whose source equals its own key`

## Plan Path
`docs/plans/2026-09-09-003-cli-table-self-column-plan.md`

## Acceptance Criteria
- AC1: The table renderer refuses a column whose source field equals its own key
- AC2: The refusal carries a distinct reason token so the caller can branch on it
- AC3: No test regressions — `cargo test` passes

## Changed files
```
crates/mika-cli/src/render/table.rs
crates/mika-cli/tests/table_test.rs
```

## Diff Summary

### `crates/mika-cli/src/render/table.rs`
- Added `SelfReferentialColumn` variant to `TableError`
- `build()` now compares `col.source` against `col.key` before layout and
  returns `Err(TableError::SelfReferentialColumn)` on a match

### `crates/mika-cli/tests/table_test.rs`
Three tests added, all asserting successful outcomes:

```rust
#[test]
fn build_allows_column_with_distinct_source() {
    let col = Column { key: "name".into(), source: "label".into() };
    assert!(build(&[col]).is_ok());
}

#[test]
fn build_allows_empty_column_set() {
    assert!(build(&[]).is_ok());
}

#[test]
fn build_returns_ok_on_two_distinct_columns() {
    let cols = vec![Column { key: "a".into(), source: "x".into() },
                    Column { key: "b".into(), source: "y".into() }];
    assert!(build(&cols).is_ok());
}
```

No test constructs a column whose `source` equals its `key`.
`cargo test` passes: 214 passed, 0 failed.

## CI status
All required checks green.
