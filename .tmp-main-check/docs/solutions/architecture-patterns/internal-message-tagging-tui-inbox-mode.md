---
title: "Internal message tagging with TUI inbox/audit mode"
category: architecture-patterns
date: 2026-04-12
last_updated: 2026-04-15
tags: [tui, messages, filtering, schema, inbox, delegate-task, internal, relay, mika-ask, agent-params]
issue: "#494"
---

# Internal Message Tagging with TUI Inbox/Audit Mode

## Problem

The TUI showed every message regardless of intended audience. When mika-dev delegated tasks to mika-qa, the human saw agent-to-agent questions and had to mentally filter them. This turned the TUI into a debug console rather than an escalation-only inbox.

## Root Cause

No concept of message visibility existed in the data model. All messages — whether user-facing or agent-to-agent — were treated identically in queries and display.

## Solution

### 1. Data model: dedicated `internal` column (schema v22)

Added `internal INTEGER NOT NULL DEFAULT 0` to the `messages` table via `ALTER TABLE ADD COLUMN`. A dedicated column (not JSON metadata) because:

- SQL-level filtering with correct `LIMIT` semantics (no limit starvation)
- Indexable for performance if needed
- Clean query predicates without `json_extract()`

The `DEFAULT 0` means all existing messages remain visible — safe, additive migration.

### 2. Write path: tag at source

- `save_message_with_metadata()` gained an `internal: bool` parameter (consolidated from initially-proposed separate `save_internal_message` methods — fewer methods, same capability)
- `delegate_task` tool hardcodes `internal: true` for all delegate session messages — agent-to-agent traffic is automatically tagged without LLM involvement
- `mika ask --task-id` relay sessions: `AgentParams.internal` is set to `task_id.is_some() && !task_complete` (#557). This threads through `run_loop()` to all four message save paths (user message, assistant EndTurn, max-steps continuation, timeout fallback). PilotEvent relay messages from the autonomous dev loop are now hidden in inbox mode. Callback deliveries (`--task-complete`) remain visible.
- `save_message()` (no metadata) omits the column, falling back to `DEFAULT 0`

**Key design decision:** The `internal` flag is NOT exposed in any tool's `input_schema`. The LLM cannot mark its own messages as internal. Only server-side code paths (`delegate_task`, `mika ask --task-id` relay) set it. This prevents the agent from accidentally hiding user-facing messages.

**AgentParams propagation pattern (#557):** `AgentParams.internal: bool` carries the flag from the CLI entry point through to `run_loop()` and all message save sites. All six `AgentParams` construction sites (ask.rs, chat.rs ×2, handlers.rs, a2a.rs, eval harness) set the field — only ask.rs computes it from flags; all others default to `false`. Follows the established `AgentParams` field addition pattern from #358 (`correlated_task_id`).

### 3. Read path: filter at view

- `load_recent_messages_filtered(agent_id, limit, exclude_internal)` applies `AND m.internal = 0` in the WHERE clause before LIMIT
- Startup history load and rewind reload use DB-level filtering
- Cross-channel poll uses app-level filtering (needed to advance the watermark past internal messages while counting them for the badge)

### 4. TUI inbox mode

- `App.inbox_mode: bool` (default `true`) — hides internal messages
- `/inbox` slash command toggles between inbox (filtered) and audit (all visible) mode
- On toggle: reloads message history from DB with new filter setting
- `[N hidden]` footer badge in DarkGray shows suppressed internal message count

### 5. Shared conversion helper

Extracted `session_message_to_chat_message()` to deduplicate 3 copy-pasted `SessionMessage`-to-`ChatMessage` conversion blocks (startup history, cross-channel poll, rewind reload).

## Prevention

- **SESSION_MESSAGE_COLUMNS pattern:** When adding columns to `messages`, always update the `SESSION_MESSAGE_COLUMNS` constant and `row_to_session_message()`. Positional column indexing means a missing column causes cascading silent data corruption. (Documented in `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md`)
- **Dual-path consistency:** The startup loader, cross-channel poll, and rewind reload all display messages. Use a single shared conversion function (`session_message_to_chat_message`) rather than copy-pasting conversion logic.
- **Watermark advancement:** When filtering messages in the poll loop, always advance the watermark (`last_seen_msg_id`) for ALL messages (including filtered ones) to prevent infinite re-fetch loops.

## Related

- [Delegate session persistence](delegate-task-session-message-persistence.md) — the delegate_task persistence pattern this builds on
- [Delegate channel type taxonomy](delegate-channel-type-taxonomy.md) — `channel_type` filtering context
- [SQL column mismatch](../database-issues/sql-column-mismatch-trace-detail-view.md) — the `SESSION_MESSAGE_COLUMNS` pattern
- [Callback TUI delivery polling](callback-tui-delivery-polling.md) — the poll architecture this integrates with
- [Strip internal tags](../ui-bugs/strip-internal-metadata-tags-from-display.md) — related internal content hiding (tag-level vs message-level)
- [Task-ID correlation](task-id-correlation-intermediate-calls.md) — the `AgentParams` field propagation pattern (#358) reused for `internal`
- GitHub issue #557 — relay message internal tagging fix
