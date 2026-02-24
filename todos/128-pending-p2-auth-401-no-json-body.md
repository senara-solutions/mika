---
status: pending
priority: p2
issue_id: "128"
tags: [code-review, agent-native, api-design]
dependencies: []
---

# 401 Auth Response Has No JSON Body

## Problem Statement

The auth middleware in `auth.rs` returns bare `StatusCode::UNAUTHORIZED` with no response body. API clients (including the gateway) receive a 401 with an empty body, making it harder to diagnose authentication issues. A JSON error body with `{"error": "unauthorized"}` would be more helpful and consistent with other error responses.

## Findings

- **Source:** agent-native-reviewer (IMPORTANT-1)
- **Location:** `crates/mika-agent/src/server/auth.rs` — returns bare StatusCode::UNAUTHORIZED
- **Evidence:** Other error responses (400, 429) include JSON bodies with error details

## Proposed Solutions

### Option 1: Return JSON error body with 401
- **Pros**: Consistent with other error responses, helpful for debugging
- **Cons**: Minimal change
- **Effort**: Trivial
- **Risk**: None

## Recommended Action

Option 1 — return `(StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})))`.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/server/auth.rs`
- **Database Changes**: None

## Acceptance Criteria

- [ ] 401 responses include JSON body with error message
- [ ] Existing auth tests updated to verify response body
- [ ] All tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** agent-native-reviewer

## Resources

- PR #5: Phase 2 Container HTTP Server
