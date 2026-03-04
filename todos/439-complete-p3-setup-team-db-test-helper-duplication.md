---
status: complete
priority: p3
issue_id: "439"
tags: [code-review, testing, duplication]
dependencies: []
---

# setup_team_db() Test Helper Duplicated

## Problem Statement

The `setup_team_db()` test helper is duplicated in `tools/get_team_history.rs` and `tools/get_team_status.rs`. The codebase already centralizes test helpers in `test_utils.rs`.

## Proposed Solutions

### Option A: Move to test_utils (Recommended)

Extract into `test_utils::test_helpers` with a flexible signature supporting both use cases.

- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Single `setup_team_db()` helper in `test_utils.rs`
- [ ] Both test modules use the shared helper
