---
status: complete
priority: p3
issue_id: "318"
tags: [code-review, quality, testing, web-search]
dependencies: []
---

# Missing Test for web_search with No API Key

## Problem Statement

The web_search handler has tests for valid queries and error cases, but no test verifying the error message when `brave_api_key` is `None` in the context. This is the most common misconfiguration scenario.

## Findings

**Source:** pattern-recognition-specialist

**Location:** `crates/mika-agent/src/skills/builtin_handlers.rs` — test module

## Proposed Solutions

### Option A: Add test_web_search_no_api_key test
- Create a test that passes `brave_api_key: None` in ToolContext and asserts the error message mentions config.toml
- **Pros:** Covers the primary misconfiguration path
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-agent/src/skills/builtin_handlers.rs`

## Acceptance Criteria

- [ ] Test exists that verifies error message when brave_api_key is None
- [ ] Error message mentions config.toml as a configuration path

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from PR #28 code review | |

## Resources

- PR: #28
