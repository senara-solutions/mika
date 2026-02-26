---
status: complete
priority: p2
issue_id: "077"
tags: [code-review, architecture, agent-native]
dependencies: []
---

# send_message should persist outbound messages and audit log

## Problem Statement
The `send_message` tool delivers messages but does not save them to conversations or log to memory_events. Messages sent during heartbeat/reminder mode are lost. The `create_reminder` and `cancel_reminder` tools also skip audit logging, breaking the pattern established by all other mutation tools.

## Findings
- send_message.rs:41-63 — no `db.save_message()` call for outbound messages
- create_reminder.rs:73-77 — no `log_memory_event()` call
- cancel_reminder.rs:41-49 — no `log_memory_event()` call
- All other mutation tools (store_fact, update_fact, update_core_memory) log to memory_events

## Proposed Solutions
### Option 1: Add persistence and audit to all three tools
```rust
// send_message: save outbound message
ctx.db.save_message("assistant", text, "outbound")?;
// create_reminder/cancel_reminder: audit log
ctx.db.log_memory_event(ctx.session_id, "create_reminder", ...)?;
```
**Effort:** 20 minutes | **Risk:** Low

## Acceptance Criteria
- [ ] send_message saves outbound messages to conversations table
- [ ] create_reminder logs to memory_events
- [ ] cancel_reminder logs to memory_events
- [ ] Tests verify persistence

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
