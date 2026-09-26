---
status: complete
priority: p1
issue_id: "235"
tags: [code-review, security, path-traversal]
dependencies: []
---

# Missing `validate_agent_name` on delete command

## Problem Statement

The `delete` function in `agents.rs` normalizes the agent name but does not call `validate_agent_name()` before using it to construct a filesystem path and calling `remove_dir_all()`. The `normalize_agent_name` function only does `trim().to_lowercase()`, so a name like `"../../../tmp"` would pass normalization and be used in path construction.

## Findings

- **Source:** Security Sentinel, Pattern Recognition Specialist
- **File:** `crates/mika-cli/src/commands/agents.rs`, `delete` function
- **Evidence:** `create` and `switch` both validate, but `delete` does not
- **Mitigating:** `agent_exists` check requires `data/mika.db` at traversed path; CLI requires local access

## Proposed Solutions

### Option A: Add validation (1-line fix) [Recommended]
Add `agent::validate_agent_name(&name)?;` after normalization, consistent with `create` and `switch`.

- **Pros:** Consistent, minimal change
- **Cons:** None
- **Effort:** Trivial
- **Risk:** None

## Acceptance Criteria

- [ ] `delete` function calls `validate_agent_name` before any filesystem operations
- [ ] Attempting to delete with a traversal name like `"../foo"` returns a validation error

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | Identified by security + pattern agents |
