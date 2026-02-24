---
status: ready
priority: p2
issue_id: "125"
tags: [code-review, quality]
dependencies: []
---

# ErrorResponse Struct Is Dead Code

## Problem Statement

`ErrorResponse` struct in `server/types.rs:20-25` is defined but never used anywhere. All error responses use inline `serde_json::json!()` instead. This is dead code that should either be used consistently or removed.

## Findings

- **Source:** code-simplicity-reviewer (CRITICAL-1), architecture-strategist, agent-native-reviewer
- **Location:** `crates/mika-agent/src/server/types.rs:20-25`
- **Evidence:** `grep -r "ErrorResponse" crates/mika-agent/src/server/` shows only the definition, no usage

## Proposed Solutions

### Option 1: Remove ErrorResponse
- **Pros**: No dead code, simpler
- **Cons**: Need to re-add if needed later
- **Effort**: Trivial
- **Risk**: None

### Option 2: Use ErrorResponse consistently in all error paths
- **Pros**: Consistent typed error responses, better for API documentation
- **Cons**: More changes needed across handlers
- **Effort**: Small
- **Risk**: Low

## Recommended Action

Option 1 — remove it. If needed later, it's trivial to re-add.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/server/types.rs`
- **Database Changes**: None

## Acceptance Criteria

- [ ] ErrorResponse struct removed
- [ ] No compiler warnings about unused code
- [ ] All tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** code-simplicity-reviewer, architecture-strategist, agent-native-reviewer

## Resources

- PR #5: Phase 2 Container HTTP Server
