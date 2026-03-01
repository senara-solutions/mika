---
status: complete
priority: p3
issue_id: 376
tags: [code-review, ci-cd, simplicity]
dependencies: []
---

# Remove redundant Build step from CI workflow

## Problem Statement

The CI workflow has a standalone `cargo build --all-targets` step between clippy and test. Both `cargo clippy --all-targets` and `cargo test` already compile the code. The explicit build step adds ~0 value due to Cargo's incremental caching but adds visual noise and marginal CI time.

## Findings

- **Source:** Code Simplicity Reviewer agent
- **Severity:** LOW — minor inefficiency/noise

## Proposed Solutions

### Option 1: Remove the Build step (Recommended)
- Delete the `Build` step from ci.yml
- clippy already compiles all targets; test compiles and runs
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `.github/workflows/ci.yml`

## Acceptance Criteria

- [ ] Build step removed from CI workflow
- [ ] CI still passes (clippy + test provide full compilation coverage)
