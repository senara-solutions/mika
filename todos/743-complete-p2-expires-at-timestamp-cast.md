---
status: pending
priority: p2
issue_id: "743"
tags: [code-review, security, robustness]
dependencies: []
---

# Fix expires_at timestamp negative cast in github_app.rs

## Problem Statement

In `exchange_jwt_for_token()`, `expires_at.timestamp() as u64` silently wraps negative values to a very large number if GitHub returns a date before Unix epoch. This would make a token appear valid for an extremely long time.

## Proposed Solutions

### Option A: Use try_into with error (Recommended)
- Replace `as u64` with `.try_into().context("expires_at before UNIX epoch")?`
- **Effort:** Small (one-line change)
- **Risk:** Low

## Acceptance Criteria

- [ ] Negative timestamps produce an error instead of wrapping
