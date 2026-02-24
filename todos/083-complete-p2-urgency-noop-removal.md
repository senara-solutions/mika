---
status: pending
priority: p2
issue_id: "083"
tags: [code-review, agent-native, quality]
dependencies: []
---

# Remove no-op urgency parameter from send_message tool

## Problem Statement
The `send_message` tool schema includes an `urgency` parameter with enum values `["high", "normal", "low"]` but the execute method completely ignores it. The description says "Currently all deliver immediately." The LLM wastes tokens deciding urgency for no effect and may form incorrect expectations about delayed delivery.

## Findings
- send_message.rs:30-34 — urgency field in schema, never read from input
- No future implementation plan exists for urgency-based routing

## Proposed Solutions
### Option 1: Remove urgency field entirely (Recommended)
Delete the urgency property from the JSON schema. Add back when it has behavior.
**Effort:** 5 minutes | **Risk:** Low

### Option 2: Keep but clarify description
Change description to "Informational only, does not affect delivery timing."
**Effort:** 2 minutes | **Risk:** Low (but still wastes LLM tokens)

## Acceptance Criteria
- [ ] urgency field removed from send_message schema
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
