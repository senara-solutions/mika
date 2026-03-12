---
status: pending
priority: p3
issue_id: 645
tags: [code-review, agent-native, tools]
dependencies: []
---

# Add trace_id to get_session_messages tool output

## Problem Statement

The `get_session_messages` tool formats messages as `[timestamp] role: content` but does not include the `trace_id`. The agent can cross-reference via `query_timeline` with trace_id filtering, but when browsing a session's messages, it cannot see which trace each message belongs to.

## Findings

- Tool at `crates/mika-agent/src/tools/get_session_messages.rs:98-107` formats output without trace_id
- Agent has `query_timeline` for trace-based queries, so this is a convenience improvement
- Flagged by: agent-native-reviewer

## Proposed Solutions

### Option A: Include trace_id in output when present
Format as `[timestamp] role (trace:abcd...): content` when trace_id is Some.
- Pros: Full introspection parity, helps agent correlate messages to traces
- Cons: Slightly longer output
- Effort: Small
- Risk: Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/tools/get_session_messages.rs`

## Acceptance Criteria

- [ ] Messages with trace_id show it in the output
- [ ] Messages without trace_id display as before
- [ ] Existing tests pass

## Work Log

- 2026-03-12: Created from code review of fix/trace-messages-endpoint branch
