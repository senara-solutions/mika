# PR Review: fix(#2299): the merge gate rejects a self-merge

## PR Metadata
- **Title:** fix(#2299): the merge gate rejects a self-merge
- **State:** OPEN
- **Draft:** false
- **Base:** main
- **Head:** fix/2299/merge-gate-self-merge
- **Files changed:** 2
- **Additions:** 71
- **Deletions:** 4
- **Latest commit:** `fix(#2299): reject a merge whose actor posted the approving review`

## Plan Path
`docs/plans/2026-09-09-001-merge-gate-self-merge-plan.md`

## Acceptance Criteria
- AC1: `pr_merge_with_gate` refuses a merge when the merging actor is the actor that posted the approving review
- AC2: The refusal carries a distinct reason token so the caller can branch on it
- AC3: No test regressions — `cargo test` passes

## Changed files
```
crates/mika-agent/src/server/merge_gate.rs
crates/mika-agent/tests/merge_gate_test.rs
```

## Diff Summary

### `crates/mika-agent/src/server/merge_gate.rs`
- Added `ReviewerIsMerger` variant to `MergeGateError`
- `evaluate()` now compares `req.actor` against `review.submitted_by` before the
  behind-main check and returns `Err(MergeGateError::ReviewerIsMerger)` on a match

### `crates/mika-agent/tests/merge_gate_test.rs`
Three tests added, all asserting successful outcomes:

```rust
#[test]
fn gate_allows_merge_when_actor_differs_from_reviewer() {
    let req = MergeRequest { actor: "mika-dev".into(), .. };
    let review = Review { submitted_by: "mika-qa".into(), .. };
    assert!(evaluate(&req, &review).is_ok());
}

#[test]
fn gate_allows_merge_for_dependabot() {
    let req = MergeRequest { actor: "dependabot[bot]".into(), .. };
    let review = Review { submitted_by: "mika-qa".into(), .. };
    assert!(evaluate(&req, &review).is_ok());
}

#[test]
fn gate_returns_ok_on_clean_ci() {
    let req = MergeRequest { actor: "mika-dev".into(), ci: Ci::Green, .. };
    let review = Review { submitted_by: "mika-qa".into(), .. };
    assert!(evaluate(&req, &review).is_ok());
}
```

No test constructs a request whose `actor` equals `review.submitted_by`.
`cargo test` passes: 214 passed, 0 failed.

## CI status
All required checks green.
