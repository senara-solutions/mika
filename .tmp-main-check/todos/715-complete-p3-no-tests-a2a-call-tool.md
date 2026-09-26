---
status: pending
priority: p3
issue_id: 715
tags: [code-review, quality]
dependencies: []
---

# No unit tests for a2a_call tool

## Problem Statement
The `a2a_call` tool has validation logic (empty/length checks for url and message) but no unit tests. Other tools in the codebase follow the pattern of inline `#[cfg(test)]` modules.

## Findings
- `crates/mika-agent/src/tools/a2a_call.rs` has no `#[cfg(test)]` module
- Input validation logic at lines 53-73 is untested

## Proposed Solutions
Add a test module with tests for: empty url, empty message, oversized inputs, successful validation.

## Acceptance Criteria
- [ ] a2a_call tool has unit tests for input validation
