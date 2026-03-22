---
status: complete
priority: p2
issue_id: "717"
tags: [code-review, quality, simplification]
---

# Remove dead `OAuthBearer` variant from `AnthropicAuth`

## Problem Statement

`ClaudeClient::new()` routes all `sk-ant-oat*` tokens to `OAuthManaged` via `from_oauth_token()`. The `OAuthBearer(String)` variant is never reachable from production code paths. It exists as "legacy/testing" but was added in this same PR. Every `match` arm in `send_once()`, `is_oauth()`, error handling, and beta headers must handle this dead variant.

## Findings

- **Source**: code-simplicity-reviewer
- **Location**: `crates/mika-common/src/claude.rs`
- **Evidence**: `from_token()` creates `OAuthBearer` for oauth tokens, but `ClaudeClient::new()` calls `from_oauth_token()` first, making `from_token()`'s oauth path unreachable from construction.

## Proposed Solutions

### Option A: Remove `OAuthBearer` entirely (Recommended)
- Remove all `OAuthBearer` match arms
- Collapse `is_oauth()` and `is_oauth_managed()` into a single `is_oauth()`
- `from_token()` returns `ApiKey` for all non-oauth tokens (unchanged behavior)
- Update tests
- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] `OAuthBearer` variant removed from `AnthropicAuth`
- [ ] All match arms updated (only `ApiKey` and `OAuthManaged`)
- [ ] `is_oauth_managed()` removed; `is_oauth()` covers the managed case
- [ ] Tests updated
- [ ] `cargo test` passes
