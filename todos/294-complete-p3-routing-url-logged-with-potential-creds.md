---
status: complete
priority: p3
issue_id: 294
tags: [code-review, security, logging]
dependencies: []
---

# `routing_url` logged in warning with potential embedded credentials

## Problem Statement

When URL parsing fails in `chat.rs:42`, the raw `routing_url` is included in the `tracing::warn!` structured log field. If the URL contains embedded credentials (e.g., `http://user:password@gateway/`), they leak to logs.

## Findings

- **Security Sentinel:** The `Debug` impl for `Settings` does not redact `routing_url`, and the warn log at `chat.rs:42` includes the full URL. Low-to-Medium impact depending on whether users embed credentials in URLs.

## Proposed Solutions

### Solution A: Strip userinfo before logging

```rust
tracing::warn!(error = %e, "invalid routing_url, skipping gateway message sender");
```
Simply remove the `url` field from the structured log, since the error message already indicates what failed.

- **Pros:** Simple, eliminates the leak vector entirely
- **Cons:** Slightly less debug info in logs
- **Effort:** Small
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-cli/src/commands/chat.rs`

## Acceptance Criteria

- [ ] Raw URL not included in log output
- [ ] Error message still indicates the problem
