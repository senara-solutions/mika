---
title: A2A Message Handlers Producing Duplicate Database Rows
category: logic-errors
date: 2026-03-20
severity: high
affected_components:
  - crates/mika-agent/src/server/a2a.rs
  - handle_message_send
  - handle_message_stream
  - SQLite messages table
tags:
  - database
  - a2a-protocol
  - message-handling
  - duplicate-writes
  - dual-write-anti-pattern
symptoms:
  - Each A2A message/send request creates 2 user rows instead of 1
  - Each A2A message/stream request creates 2 assistant rows instead of 1
  - Duplicate rows differ only in metadata (trace_id vs a2a_message_id)
---

# A2A Dual-Write Producing Duplicate Message Rows

## Problem

A single A2A `message/send` or `message/stream` call produced **duplicate rows** in the `messages` table — both the user message and assistant response appeared twice. Two independent code paths each inserted the same message:

- **User message:** `a2a_insert_message` (handler) + `save_message` (`run_agent`)
- **Assistant message:** `a2a_insert_message` (handler) + `save_message_with_metadata` (`run_agent`)

The duplicates were distinguishable by metadata: the handler-written rows had A2A metadata (`a2a_message_id`, `a2a_parts`, `a2a_task_id`) but no `trace_id`; the `run_agent`-written rows had `trace_id` but no A2A metadata.

## Root Cause

Classic **dual-write anti-pattern**. Two layers both assumed persistence ownership:

1. **A2A handler layer** called `a2a_insert_message()` directly for both inbound user and outbound assistant messages
2. **Business logic layer** (`run_agent`, invoked via `run_a2a_agent`) independently called `save_message()` and `save_message_with_metadata()`

Both paths wrote to the same `messages` table in the same session, with no deduplication constraint.

## Solution

Removed all 4 `a2a_insert_message` calls from both handlers. `run_agent` is the single persistence owner.

### `handle_message_send` (3 changes)

1. Removed user message insert (`a2a_insert_message(&task_id, &params.message)`)
2. Removed assistant `Message` struct construction + `a2a_insert_message(&task_id, &response_message)`
3. Changed `Ok(response_text)` to `Ok(_)` — value no longer used

### `handle_message_stream` (2 changes)

4. Removed user message insert (`a2a_insert_message(&task_id, &params.message)`)
5. Removed assistant message insert (`a2a_insert_message(&task_id_clone, &response_message)`)
   - `response_message` struct retained — still needed for SSE `StatusUpdate` completion event

### Data cleanup

```sql
DELETE FROM messages WHERE id IN (2690, 2693);
```

Rows 2690 (duplicate user, no trace_id) and 2693 (duplicate assistant, no trace_id) were the handler-written duplicates. Rows 2691+2692 (with trace_id) were the correct `run_agent`-written rows.

## Known Limitations

Messages saved by `run_agent` lack A2A metadata (`a2a_message_id`, `a2a_parts`, `a2a_task_id`):

- Client `messageId` doesn't round-trip (returns as empty string in `a2a_get_messages` fallback path)
- Original A2A parts structure is lost (reconstructed as plain text from `content` column)

**Acceptable now** — only text parts are supported. Revisit when adding `FilePart`/`DataPart` support. The right fix at that point is a `save_user_message: bool` flag on `AgentParams` so the A2A handler controls persistence at the call site.

## Verification

- `cargo test`: 1049 passed, 0 failed
- `cargo clippy`: clean
- Manual `message/send` curl: exactly 1 user + 1 assistant row, same `trace_id`
- Manual `message/stream` curl: exactly 1 user + 1 assistant row, SSE events correct (working → completed with message)

## Prevention

### Architectural principle

Designate ONE layer as the persistence owner for each entity type. For messages, that's the business logic layer (`run_agent`). Handlers are thin adapters — they translate protocol to business logic calls, never write to DB directly.

### Code review checklist

- [ ] Does the handler import or call DB write functions directly? (Red flag)
- [ ] Trace the full call chain: request → handler → business logic → DB. Are there multiple write paths?
- [ ] If handler and business logic both touch the same table, is there a clear reason and deduplication?

### Testing

Integration tests should assert row counts after a full handler → agent loop → DB roundtrip, not just test each layer in isolation.

## Related

- [A2A Protocol Implementation](../integration-issues/a2a-protocol-implementation.md) — comprehensive A2A v0.3 implementation guide
- [Agent Creates Duplicates After Compaction](agent-creates-duplicates-after-compaction.md) — three-layer defense against duplicates (prompt + DB constraint + tool fallback)
- [Team Engine Code Review Findings](team-engine-code-review-findings-batch.md) — prior instance of silent `let _ =` on DB writes
