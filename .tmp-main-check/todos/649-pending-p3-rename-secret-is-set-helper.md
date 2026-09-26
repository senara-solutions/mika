---
status: pending
priority: p3
issue_id: 649
tags: [code-review, quality]
dependencies: []
---

# Rename `secret_is_set` helper to `env_is_set`

## Problem Statement

The `secret_is_set` function in `setup.rs` simply checks if an env var is set
and non-empty. It is now used for both secret keys (`MIKA_INVESTIGATE_GITHUB_TOKEN`)
and non-secret keys (`MIKA_GITHUB_REPO`), making the name misleading.

## Findings

- `crates/mika-cli/src/commands/setup.rs:424` — function definition
- `crates/mika-cli/src/commands/setup.rs:139` — used for non-secret `MIKA_GITHUB_REPO`
- Related: `todos/610-pending-p2-remove-get-env-var-simplification.md` (existing todo about env var helpers)

Detected by: architecture-strategist, pattern-recognition-specialist, security-sentinel

## Proposed Solutions

### Option A: Rename to `env_is_set`
- Simple rename, all call sites updated
- **Pros:** Accurate naming
- **Cons:** Minor churn
- **Effort:** Small
- **Risk:** None

### Option B: Leave as-is
- Function works correctly regardless of name
- **Pros:** No change
- **Cons:** Misleading name persists
- **Effort:** None

## Recommended Action

Option A, or consolidate with existing todo #610.

## Technical Details

- **Affected files:** `crates/mika-cli/src/commands/setup.rs`

## Acceptance Criteria

- [ ] Function name accurately describes its behavior
- [ ] All call sites updated

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Pre-existing naming issue, surfaced by new non-secret usage |

## Resources

- Related: todo #610
