---
status: pending
priority: p2
issue_id: "326"
tags: [code-review, ux, telegram]
dependencies: []
---

# Stale "text-only" Unsupported Message

## Problem Statement

The `Unsupported` handler in `crates/mika-gateway/src/routes.rs:178` says "I can only read text messages right now" which is now factually incorrect since this PR adds image support. Users sending stickers, voice, or video get a misleading rejection. The comment on line 173 is also stale.

## Findings

- Flagged by: simplicity-reviewer, agent-native-reviewer, architecture-strategist
- Location: `crates/mika-gateway/src/routes.rs:173-180`
- Also stale: `docs/deployment.md:946-954` which says photos trigger the text-only error

## Proposed Solutions

### Option A: Update message and comment
- **Pros:** Simple, accurate
- **Cons:** None
- **Effort:** Small
- **Risk:** None

Update comment to "non-image media (sticker/voice/video/etc.)" and message to "I can read text and image messages. This media type isn't supported yet."

## Acceptance Criteria

- [ ] Unsupported handler message reflects image support
- [ ] Comment on line 173 updated
- [ ] deployment.md updated
