---
status: complete
priority: p1
issue_id: "237"
tags: [code-review, security, input-validation]
dependencies: []
---

# `read_active_agent` returns unvalidated content

## Problem Statement

`read_active_agent` reads the `active_agent` file and returns raw content without validation. If the file is tampered with (e.g., contains `"../../../etc"`), the unvalidated value propagates to `resolve_agent_home` and path construction in the CLI path.

## Findings

- **Source:** Security Sentinel
- **File:** `crates/mika-common/src/home.rs` (`read_active_agent`), `crates/mika-cli/src/init.rs` (`resolve_active_agent`)
- **Evidence:** No call to `validate_agent_name` on the return value anywhere in the chain

## Proposed Solutions

### Option A: Validate in `resolve_active_agent` [Recommended]
Add `agent::validate_agent_name(&name).context("active_agent file contains invalid agent name")?;` in `resolve_active_agent()`.

- **Pros:** Central validation point, all CLI paths go through this
- **Cons:** None
- **Effort:** Trivial (2-3 lines)
- **Risk:** None

## Acceptance Criteria

- [ ] `resolve_active_agent` validates the name before returning
- [ ] A poisoned `active_agent` file with traversal characters returns an error

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | File poisoning defense-in-depth |
