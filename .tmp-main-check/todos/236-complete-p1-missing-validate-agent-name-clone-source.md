---
status: complete
priority: p1
issue_id: "236"
tags: [code-review, security, path-traversal]
dependencies: []
---

# Missing `validate_agent_name` on clone source name

## Problem Statement

The `clone` function validates the target name but does not validate the source name. An unvalidated source name is used to construct paths for file reads via `agent_dir()`, potentially allowing reads from outside the agents directory.

## Findings

- **Source:** Security Sentinel, Agent-Native Reviewer
- **File:** `crates/mika-cli/src/commands/agents.rs`, `clone` function
- **Evidence:** Target has `validate_agent_name(&target)?;` but source only gets `normalize_agent_name`

## Proposed Solutions

### Option A: Add validation (1-line fix) [Recommended]
Add `agent::validate_agent_name(&source)?;` after normalization.

- **Pros:** Consistent with target validation
- **Cons:** None
- **Effort:** Trivial
- **Risk:** None

## Acceptance Criteria

- [ ] `clone` function validates both source and target names
- [ ] Attempting to clone from `"../../etc"` returns a validation error

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | Asymmetric validation found by security agent |
