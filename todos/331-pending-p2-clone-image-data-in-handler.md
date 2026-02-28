---
status: pending
priority: p2
issue_id: "331"
tags: [code-review, performance]
dependencies: []
---

# Image Data Cloned Instead of Moved in Agent Handler

## Problem Statement

In `handle_message()`, `img.data.clone()` copies the ~6.7MB base64 string when converting `ImagePayload` to `ImageSource`. Since `req.images` is consumed by the handler, the data could be moved instead of cloned, saving a 6.7MB allocation per image.

## Findings

- Flagged by: performance-oracle, agent-native-reviewer
- Location: `crates/mika-agent/src/server/handlers.rs:120-130`

## Proposed Solutions

### Option A: Use take() + into_iter() to move
- **Pros:** Eliminates 6.7MB allocation per image
- **Cons:** Requires mut binding for req
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Image data moved instead of cloned
- [ ] No unnecessary memory allocation for image conversion
