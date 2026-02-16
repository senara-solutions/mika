---
status: complete
priority: p3
issue_id: 269
tags: [code-review, architecture, api-design]
dependencies: []
---

# Make run_agent return Option<String> instead of String

## Problem Statement

`run_agent` returns `Ok("")` for tool-use-only turns (no text blocks). Both CLI (`app.rs:220`) and server (`handlers.rs:142`) independently guard against empty strings. The empty case should be explicit in the type system.

## Findings

- **File**: `crates/mika-agent/src/agent.rs:189-195`
- **Impact**: Low — both consumers already handle it correctly
- **Found by**: agent-native-reviewer

## Proposed Solutions

### Option A: Change return type to Option<String> (Recommended)
- Return `None` for tool-use-only turns, `Some(text)` for text responses
- Pros: Type-safe, eliminates independent empty-string guards
- Cons: Touches all call sites (CLI, server, ask command)
- Effort: Medium
- Risk: Low

## Acceptance Criteria

- [ ] `run_agent` returns `Result<Option<String>>`
- [ ] CLI and server match on `Some(text)` / `None`
- [ ] All tests pass

## Work Log

| Date | Action | Notes |
|------|--------|-------|
| 2026-02-25 | Created | Found during PR #16 review |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/16
