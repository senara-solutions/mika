---
status: pending
priority: p3
issue_id: "135"
tags: [code-review, api-design]
dependencies: []
---

# Axum Default Rejection Returns Plain Text Instead of JSON

## Problem Statement

When Axum fails to deserialize a request body (e.g., malformed JSON), it returns a plain text error response. For a JSON API, all error responses should be JSON for consistency.

## Findings

- **Source:** agent-native-reviewer
- **Location:** Axum default behavior for `Json<T>` extraction failures

## Proposed Solutions

### Option 1: Add custom rejection handler for Json extractor
- **Pros**: Consistent JSON error responses
- **Cons**: Slightly more boilerplate
- **Effort**: Small
- **Risk**: Low

## Acceptance Criteria

- [ ] Malformed JSON requests return JSON error responses
- [ ] Error format matches other error responses

## Work Log

### 2026-02-24 - Identified during PR #5 review

## Resources

- PR #5: Phase 2 Container HTTP Server
