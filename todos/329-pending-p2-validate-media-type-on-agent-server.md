---
status: pending
priority: p2
issue_id: "329"
tags: [code-review, security, validation]
dependencies: []
---

# No media_type Validation on Agent Server

## Problem Statement

The agent server accepts arbitrary `media_type` strings in `ImagePayload` without validation. While the gateway validates via magic bytes, the `/message` API is a distinct trust boundary. A compromised token or direct API caller could inject invalid media types that cause Claude API errors deep in the agent loop instead of a clean 400 at the boundary.

## Findings

- Flagged by: security-sentinel (Medium), agent-native-reviewer
- Location: `crates/mika-agent/src/server/handlers.rs:120-130`, `crates/mika-agent/src/server/types.rs:6-9`

## Proposed Solutions

### Option A: Allowlist validation in handler
- **Pros:** Defense in depth, clean 400 errors
- **Cons:** Minor code addition
- **Effort:** Small
- **Risk:** None

Validate against `["image/jpeg", "image/png", "image/gif", "image/webp"]`.

## Acceptance Criteria

- [ ] Invalid media_type returns 400
- [ ] Valid types pass through
