---
status: pending
priority: p2
issue_id: "044"
tags: [code-review, performance, rust-v2]
dependencies: []
---

# send_message Clones Entire Request Each Iteration

## Problem Statement
The agent loop calls `claude.send_message(request.clone())` on every iteration, deep-cloning the system prompt (~4KB), tool definitions, and the entire growing message history. By iteration 5, this clones 20-30KB. The method takes ownership but only serializes to JSON.

**Location:** `crates/mika-agent/src/agent.rs:126`, `crates/mika-common/src/claude.rs:161`

**Reported by:** performance-oracle

## Proposed Solutions

### Option A: Change send_message to take &MessagesRequest (Recommended)
The method only serializes via `self.client.post(API_URL).json(request)`, which works with a reference.
- **Pros:** Zero-copy, simple signature change
- **Cons:** Minor API change
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria
- [ ] `send_message` takes `&MessagesRequest` instead of `MessagesRequest`
- [ ] No `.clone()` needed in agent loop
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | Code comments already acknowledge this was intended |
