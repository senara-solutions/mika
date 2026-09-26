---
status: pending
priority: p3
issue_id: "744"
tags: [code-review, security]
dependencies: []
---

# Truncate GitHub API error body in log messages

## Problem Statement

When installation token exchange fails, the full GitHub API response body is included in the error message which propagates to `warn!` logs via `resolve_github_token()`. This could expose app context or API details in logs.

## Proposed Solutions

### Option A: Truncate body in error message
- Limit error body to first 200 chars in the bail! message
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Error body truncated in the bail! message or logged at debug level only
