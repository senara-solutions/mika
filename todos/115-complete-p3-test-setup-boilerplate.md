---
status: complete
priority: p3
issue_id: "115"
tags: [code-review, quality, testing]
dependencies: []
---

# Repeated Test Setup Boilerplate Across Tool Files

## Problem Statement

Every tool test function repeats the same 3-line setup pattern ~50 times across 8 tool files:

```rust
let db = test_async_db();
let counter = AtomicU32::new(0);
let ctx = test_ctx(&db, &counter);
```

The `AtomicU32::new(0)` is boilerplate noise in tests that never inspect the counter.

## Findings

- **Source:** pattern-recognition-specialist (2B)
- **Location:** All 8 tool files under `crates/mika-agent/src/tools/`
- **Evidence:** ~150 lines of duplicated setup code

## Proposed Solutions

### Option 1: TestHarness struct in test_utils (Recommended)
- **Pros**: One-line setup, owns all values, solves lifetime issues
- **Cons**: Slightly more infrastructure code
- **Effort**: Small

```rust
pub struct TestHarness {
    pub db: AsyncDatabase,
    pub counter: AtomicU32,
}
impl TestHarness {
    pub fn new() -> Self { ... }
    pub fn ctx(&self) -> ToolContext<'_> { ... }
}
```

### Option 2: Helper function returning tuple
- **Pros**: Minimal change
- **Cons**: Tuple destructuring is still verbose, lifetime issues with references
- **Effort**: Small
- **Risk**: Low

## Recommended Action

_To be filled during triage_

## Technical Details

- **Affected Files**: `crates/mika-agent/src/test_utils.rs`, all 8 tool test modules

## Acceptance Criteria

- [ ] Tool tests use single-line setup
- [ ] All 132 tests pass
- [ ] No test readability regression

## Work Log

### 2026-02-24 - Identified in v4 Code Review
**By:** Multi-agent review (pattern-recognition-specialist)

## Resources

- Commit under review: 38a843b
