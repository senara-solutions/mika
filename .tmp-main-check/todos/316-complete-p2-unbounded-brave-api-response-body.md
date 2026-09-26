---
status: complete
priority: p2
issue_id: "316"
tags: [code-review, security, web-search]
dependencies: []
---

# Unbounded Brave API Response Body in web_search

## Problem Statement

The `web_search` builtin handler reads the full Brave API response body without any size limit before JSON parsing. A malicious, misconfigured, or compromised API endpoint could return an arbitrarily large response, causing memory exhaustion.

## Findings

**Source:** security-sentinel, performance-oracle

**Location:** `crates/mika-agent/src/skills/builtin_handlers.rs` — `web_search()` function

The handler calls `response.text().await?` which reads the entire body into memory. There is no Content-Length check or body size limit.

## Proposed Solutions

### Option A: Limit response body size via reqwest
- Use `response.bytes()` with a manual size check, or configure reqwest with a body size limit
- **Pros:** Simple, defense in depth
- **Cons:** Requires choosing a reasonable limit
- **Effort:** Small
- **Risk:** Low

### Option B: Stream and parse with size limit
- Stream the response body with a cap (e.g., 1MB) before parsing JSON
- **Pros:** Most robust
- **Cons:** More code complexity
- **Effort:** Medium
- **Risk:** Low

## Recommended Action

Option A — add a 1MB body size limit check before JSON parsing.

## Technical Details

- **Affected files:** `crates/mika-agent/src/skills/builtin_handlers.rs`
- **Components:** web_search builtin handler

## Acceptance Criteria

- [ ] Response body is limited to a reasonable size (e.g., 1MB)
- [ ] Oversized responses return a clear error message
- [ ] Existing tests still pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from PR #28 code review | New web_search handler introduced in this PR |

## Resources

- PR: #28
