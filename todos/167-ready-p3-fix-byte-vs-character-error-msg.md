---
status: complete
priority: p3
issue_id: "167"
tags: [code-review, quality]
---

# Fix Byte vs Character Length Check in /send Validation

## Problem Statement
`payload.text.len()` in routes.rs:298 checks byte length, but the error message says "1-50000 characters". Multi-byte UTF-8 text (e.g., CJK, emoji) could be rejected incorrectly or allowed larger than intended.

## Findings
- **Security sentinel**: LOW — UX inconsistency for non-ASCII messages

## Proposed Solutions

### Option A: Change error message to say "bytes" (Recommended)
Simplest fix — document the actual behavior:
```rust
Json(serde_json::json!({"error": "text must be 1-50000 bytes"}))
```
- Effort: Trivial (2 min)
- Risk: None

### Option B: Use .chars().count() for true character count
- Effort: Small — but adds CPU cost per request
- Risk: Low

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs` (line 301)

## Acceptance Criteria
- [ ] Error message accurately describes the validation

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
