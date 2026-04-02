---
status: pending
priority: p3
issue_id: 746
tags: [code-review, gateway, quality]
dependencies: []
---

# GitHub webhook format_event_text body truncation at char boundary

## Problem Statement

The `format_event_text` function truncates issue/PR/comment bodies at 2000 chars using `.chars().take(2000).collect::<String>()`. While this is UTF-8 safe, it doesn't indicate to the agent that the body was truncated, potentially leading the agent to act on incomplete information.

## Findings

- Location: `crates/mika-gateway/src/github.rs`, format_event_text function
- Multiple `.chars().take(2000).collect::<String>()` calls across event types
- No truncation indicator appended (e.g., "... [truncated]")

## Proposed Solutions

### Option 1: Add truncation indicator (Recommended)
After truncation, append `"\n\n[truncated — full text at URL]"` when the original exceeds 2000 chars.

**Pros:** Agent knows the body was truncated, can follow URL for full text
**Cons:** Minimal code change
**Effort:** Small
**Risk:** None

## Acceptance Criteria

- [ ] Truncated bodies include a truncation indicator
- [ ] The indicator references the URL where full text is available

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-04-02 | Created from code review of #382 | |
