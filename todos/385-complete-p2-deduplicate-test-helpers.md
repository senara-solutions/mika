---
status: complete
priority: p2
issue_id: 385
tags: [code-review, quality, testing]
dependencies: []
---

# Deduplicate test helpers across management tool tests

## Problem Statement

`dummy_settings()` is copy-pasted identically in `delegate_task.rs:159` and `run_team.rs:92` (17 lines each). `test_run()` is nearly identical across `get_team_status.rs:131` and `teams/history.rs:95`. This violates DRY in test code.

## Findings

- **Source:** Code Simplicity Reviewer
- ~34 lines of duplicated test code across 4 files
- Both helpers use the same field values

## Proposed Solutions

### Option 1: Extract to test_utils module (Recommended)
Add `pub fn dummy_settings() -> Settings` and `pub fn test_team_run() -> TeamRun` to `test_utils::test_helpers`.
- **Effort:** Small
- **Risk:** None
- **Saves:** ~34 lines
