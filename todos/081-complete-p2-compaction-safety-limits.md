---
status: complete
priority: p2
issue_id: "081"
tags: [code-review, performance, architecture]
dependencies: []
---

# Add safety limits to conversation compaction

## Problem Statement
Compaction sends all old messages to Claude in a single API call with no size guard. If compaction fails repeatedly (API errors), messages accumulate. 200 messages at 500 chars each = 100KB payload risking context window overflow. The conversation summary also grows unbounded in the system prompt.

## Findings
- compaction.rs:30-45 — loads ALL old messages, no cap
- agent.rs:91-97 — summary appended to system prompt, grows each compaction
- Summarization prompt says "under 500 tokens" but not enforced in code
- Growing system prompt defeats Claude API prompt caching

## Proposed Solutions
### Option 1: Cap batch size + enforce summary length
- Limit compaction to 100 messages per batch
- Truncate or re-summarize if summary exceeds 500 tokens (~2000 chars)
- Log warning when payload exceeds size threshold
**Effort:** 1 hour | **Risk:** Low

### Option 2: Move summary to conversation history
Place summary as first message instead of system prompt to preserve prompt caching.
**Effort:** 30 minutes | **Risk:** Medium (behavior change)

## Acceptance Criteria
- [ ] Compaction batches limited to reasonable max
- [ ] Summary length enforced (truncation or re-summarization)
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
