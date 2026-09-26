---
status: complete
priority: p2
issue_id: "676"
tags: [code-review, testing, security]
dependencies: []
---

# Missing test coverage for reference workspace fallback

## Problem Statement

All existing tests for `read_workspace` and `list_workspace` pass `reference_dir: None`. The fallback logic (try current workspace, fall back to reference) and the two-section listing are completely untested. This is security-relevant since the reference directory involves path resolution across two directory trees.

## Findings

- **Architecture Strategist**: Medium severity. No test verifies the security boundary between workspace trees.
- **Pattern Recognition**: Missing tests noted for `reference_dir` fallback path.
- **Code Simplicity**: The `!output.is_error` proxy for "file not found" in the fallback logic is untested.

**Affected files:**
- `crates/mika-agent/src/tools/read_workspace.rs` (tests section)
- `crates/mika-agent/src/tools/list_workspace.rs` (tests section)

## Proposed Solutions

### Option A: Add targeted tests for reference_dir scenarios (Recommended)
Add tests for:
1. File found in current workspace only (no fallback)
2. File found only in reference workspace (fallback exercised)
3. File in both (current wins)
4. Path traversal attempt against reference dir
5. `list_workspace` with both dirs populated (two-section output)
- **Pros:** Comprehensive, verifies security boundary
- **Cons:** ~80 lines of test code
- **Effort:** Medium
- **Risk:** None

## Acceptance Criteria

- [ ] Test: file in current only → returns current content
- [ ] Test: file in reference only → returns reference content
- [ ] Test: file in both → current wins
- [ ] Test: path traversal against reference dir → rejected
- [ ] Test: list_workspace shows both sections with labels

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-15 | Created from code review | Architecture + pattern recognition flagged |

## Resources

- Architecture strategist finding F
- Pattern recognition missing test coverage section
