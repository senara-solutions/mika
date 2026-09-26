---
status: complete
priority: p3
issue_id: "721"
tags: [code-review, simplification]
---

# Inline `create_token_manager()` and reduce `force_refresh()` visibility

## Problem Statement

`create_token_manager()` is a one-line wrapper (`Arc::new(OAuthTokenManager::new(...))`) with a single call site. `force_refresh()` is `pub` but only called from within `mika-common`.

## Proposed Solutions

- Inline `create_token_manager()` at the call site in `AnthropicAuth::from_oauth_token()`
- Change `force_refresh()` from `pub` to `pub(crate)`
- **Effort**: Small (remove 4 lines, change 1 visibility keyword)

## Acceptance Criteria

- [ ] `create_token_manager()` removed, inlined at call site
- [ ] `force_refresh()` changed to `pub(crate)`
- [ ] `cargo test` passes
