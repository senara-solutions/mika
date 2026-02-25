---
status: pending
priority: p3
issue_id: 264
tags: [code-review, quality, patterns]
dependencies: []
---

# Add Missing Path Length Check in read_workspace

## Problem Statement

`read_workspace.rs` does not validate the path input against `MAX_INPUT_LEN`. All other tools consistently validate string inputs against the 10,000 character limit. While path traversal is blocked, an extremely long path string could still be passed without rejection.

## Findings

- All tools in the codebase validate string inputs with both an empty check and a `MAX_INPUT_LEN` (10,000 chars) check.
- `read_workspace.rs` performs the empty check but is missing the `MAX_INPUT_LEN` check.
- `write_workspace.rs` correctly validates the path against `MAX_INPUT_LEN`, showing this is the intended pattern.
- While an extremely long path is unlikely to cause a security issue (path traversal is separately blocked), it breaks consistency with other tools and could waste resources on a clearly invalid input.

## Proposed Solutions

Add a `MAX_INPUT_LEN` check after the existing empty check in `read_workspace.rs`, following the same pattern used in `write_workspace.rs`.

```rust
use crate::tools::MAX_INPUT_LEN;

// After the empty check:
if path.len() > MAX_INPUT_LEN {
    return Ok(ToolOutput::error(
        "Path exceeds maximum length".to_string(),
    ));
}
```

## Technical Details

**Files affected:**
- `crates/mika-agent/src/tools/read_workspace.rs`

**Changes required:**
1. Add `MAX_INPUT_LEN` to the imports from `crate::tools`
2. Add the length validation check after the existing empty string check
3. Add a test case for an oversized path input

## Acceptance Criteria

- [ ] `MAX_INPUT_LEN` imported in read_workspace.rs
- [ ] Path input validated against `MAX_INPUT_LEN` after empty check
- [ ] Error message is consistent with other tools
- [ ] Test added for oversized path input
- [ ] All tests pass (`cargo test`)

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from code review of PR #13 |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
