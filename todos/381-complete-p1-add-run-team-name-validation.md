---
status: complete
priority: p1
issue_id: 381
tags: [code-review, security, consistency]
dependencies: []
---

# Add validate_team_name() to RunTeamTool

## Problem Statement

All other team tools (`get_team_status`, `get_team_history`) call `team::validate_team_name()` to reject malformed names, but `run_team` skipped this validation. This was a defense-in-depth gap since `run_team` constructs filesystem paths from the team name.

## Findings

- **Source:** Agent-Native Reviewer
- **File:** `crates/mika-agent/src/tools/run_team.rs:42-64`
- Inconsistent with validation pattern used across all other tools

## Resolution

Added `team::validate_team_name()` check after the length check, plus a test for invalid names.

## Work Log

- 2026-03-02: Added validation and test_run_team_invalid_name test
