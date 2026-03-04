---
status: complete
priority: p1
issue_id: "441"
tags: [code-review, security, panic]
dependencies: []
---

# UTF-8 Truncation Panic in Task Description

## Problem Statement

In `crates/mika-agent/src/teams/engine.rs:888-890`, task descriptions exceeding 5000 chars are truncated with `task[..5000]`, which panics if byte 5000 falls within a multi-byte UTF-8 character.

## Findings

- `task[..5000].to_string()` will panic at runtime on multi-byte boundaries
- The `floor_char_boundary` pattern is already used at `prompt.rs:41` and `get_team_status.rs`

## Fix

Use `task[..task.floor_char_boundary(5000)].to_string()`.

## Acceptance Criteria

- [ ] Truncation uses `floor_char_boundary`
- [ ] Test added for multi-byte boundary case
