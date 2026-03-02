---
status: complete
priority: p2
issue_id: 383
tags: [code-review, security, consistency]
dependencies: []
---

# Add run_id length validation in get_team_status

## Problem Statement

The `run_id` parameter in `get_team_status` has no length check, inconsistent with the `team_name` parameter in the same function which validates against `MAX_INPUT_LEN`. While only used for an in-memory comparison, this is a validation gap.

## Findings

- **Source:** Security Sentinel
- **File:** `crates/mika-agent/src/tools/get_team_status.rs:58`
- Also missing a test for the `run_id` lookup path (lines 61-69)

## Proposed Solutions

### Option 1: Add length check and test (Recommended)
Add `if id.len() > 128 { return error }` after extracting `run_id`, plus a test for the run_id lookup path.
- **Effort:** Small
- **Risk:** None
