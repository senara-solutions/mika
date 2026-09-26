---
status: complete
priority: p2
issue_id: "317"
tags: [code-review, performance, web-search]
dependencies: []
---

# reqwest::Client Created Per web_search Call

## Problem Statement

The `web_search` handler creates a new `reqwest::Client` on every invocation. This incurs TLS handshake, connection pool setup, and DNS resolution overhead on each call. The client should be shared or cached.

## Findings

**Source:** performance-oracle, architecture-strategist

**Location:** `crates/mika-agent/src/skills/builtin_handlers.rs` — `web_search()` function

```rust
let client = reqwest::Client::new();
```

This is a known pattern issue in the codebase (see completed todos #121, #214, #290). The web_search handler is new code that repeats this anti-pattern.

## Proposed Solutions

### Option A: Add reqwest::Client to ToolContext
- Thread a shared `reqwest::Client` through `ToolContext` alongside other shared resources
- **Pros:** Consistent with existing threading pattern, connection pooling across all tools
- **Cons:** Yet another field on ToolContext (growing field count)
- **Effort:** Medium (threading through many construction sites)
- **Risk:** Low

### Option B: lazy_static / once_cell client in builtin_handlers
- Create a module-level shared client
- **Pros:** Minimal code changes, self-contained
- **Cons:** Less configurable, not shared across tools
- **Effort:** Small
- **Risk:** Low

## Recommended Action

Option B for now — module-level `std::sync::LazyLock<reqwest::Client>` in builtin_handlers. Option A can be done as a broader refactor later.

## Technical Details

- **Affected files:** `crates/mika-agent/src/skills/builtin_handlers.rs`

## Acceptance Criteria

- [ ] reqwest::Client is reused across web_search calls
- [ ] Existing tests still pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from PR #28 code review | Related to completed todos #121, #214, #290 |

## Resources

- PR: #28
