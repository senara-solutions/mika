---
title: "Tool Call Introspection: Persist and Surface Tool Summaries in Conversation History"
date: 2026-02-27
category: logic-errors
tags:
  - tool-calls
  - conversation-history
  - sqlite
  - metadata
  - compaction
  - introspection
  - agent-loop
severity: medium
modules:
  - crates/mika-agent/src/agent.rs
  - crates/mika-agent/src/db.rs
  - crates/mika-agent/src/async_db.rs
  - crates/mika-agent/src/compaction.rs
related:
  - docs/solutions/refactoring/agent-loop-variant-extraction-and-deduplication.md
  - docs/solutions/architecture/async-database-wrapper-pattern.md
  - docs/solutions/logic-errors/skill-availability-and-send-message-honesty.md
---

# Tool Call Introspection: Cross-Turn Persistence

## Problem Statement

The Mika agent had no durable record of tool calls once a conversation turn ended. Tool execution details (name, inputs, outputs) existed only in transient in-memory structures during the active turn and were not written to the database. When a user asked the agent to recall what it had done (e.g., "what tmux command did you just send?"), the agent could not answer because the information had been discarded. Compaction and conversation replay also omitted all tool activity from prior turns.

## Root Cause Analysis

Tool call results were handled solely within the agent loop's in-memory turn context and were never mapped to a persistable representation before the turn completed. The existing `conversations` table had a `metadata TEXT` column (since schema v4) capable of holding arbitrary JSON, but no code path wrote tool summaries into it or read them back when reconstructing conversation history.

## Investigation Steps

1. Traced the agent loop in `agent.rs` — `process_tool_calls()` dispatched tools but returned nothing
2. Checked `save_message()` in `db.rs` — only saved role, content, channel_type; metadata column unused
3. Checked `load_recent_messages()` — built plain `MessageContent::Text` messages, no metadata
4. Confirmed the `metadata TEXT` column existed in schema since v4 but was never written to
5. Identified six secondary defects during code review (see below)

## Working Solution

### Core Data Structure

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallSummary {
    pub step: u32,
    pub name: String,
    pub input_summary: String,   // truncated to 120 chars
    pub output_summary: String,  // truncated to 180 chars
    pub success: bool,
}
```

### Data Flow

1. **Capture**: `process_tool_calls()` returns `Vec<ToolCallSummary>` alongside existing side effects
2. **Accumulate**: `run_loop()` collects summaries across all steps via `all_tool_summaries.extend(step_summaries)`
3. **Persist**: On `EndTurn`, `save_message_with_metadata()` writes JSON to the `metadata` column
4. **Load**: `load_recent_messages()` now SELECT includes the `metadata` column
5. **Surface**: History builder appends `[Tools used: tool_name(input) → output]` blocks to assistant messages
6. **Compact**: `extract_tool_names()` includes tool names in compaction summarization input

### Metadata JSON Schema

```json
{
  "tool_calls": [
    {
      "step": 0,
      "name": "tmux_send_command",
      "input_summary": "{\"session\":\"mika\",\"text\":\"cargo test\"}",
      "output_summary": "Command sent to session 'mika'",
      "success": true
    }
  ]
}
```

### Constraints

| Field | Limit | Enforcement |
|---|---|---|
| `input_summary` | 120 chars | `truncate_summary()` at capture time |
| `output_summary` | 180 chars | `truncate_summary()` at capture time |
| Total metadata JSON | 4000 bytes | `tool_calls_metadata_json()` drops trailing entries |
| Truncation safety | UTF-8 char boundaries | `is_char_boundary()` walk-back |

### History Block Example

```
[Tools used: tmux_send_command({"session":"mika","text":"cargo test --lib"}) → Command sent; search_memory({"query":"meetings"}) → Found 3 results]
```

## Code Review Findings (6 issues found and fixed)

1. **P1 CRITICAL**: `truncate_summary` used byte-index slicing that panics on multi-byte UTF-8. Fixed with `is_char_boundary()` walk-back.
2. **P2**: `TOOL_METADATA_MAX` cap not enforced after second-pass truncation. Fixed by lowering per-field limits and adding entry-count cap.
3. **P2**: `input_summary` stored but not surfaced in history block. Fixed to include input in the rendered format.
4. **P2**: `max_steps_exceeded` fallback dropped accumulated tool summaries. Fixed to use `save_message_with_metadata`.
5. **P3**: `save_message` and `save_message_with_metadata` were independent INSERT paths. Fixed by making the former delegate to the latter.
6. **P3**: `format_tool_summary_block` used `?` that silently dropped partial results. Fixed with `filter_map`.

## Prevention Strategies

### UTF-8 Safety in String Manipulation

- **Never use byte-index slicing for user-visible truncation.** The pattern `&s[..max_bytes]` panics if the index falls inside a multi-byte character. Always use `is_char_boundary()` or `char_indices()`.
- **Test with multi-byte characters in every string-manipulation test.** Include 2-byte (é), 3-byte (中), and 4-byte (🐙) characters at truncation boundaries.
- **Existing correct pattern**: `compaction.rs` already used `while !s.is_char_boundary(s.len())` — follow established patterns.

### Enforcing Constants and Invariants

- **Co-locate enforcement with the constant.** A size cap constant without a corresponding check is a lie.
- **Write enforcement first, test second.** When introducing a cap, write the truncation logic simultaneously.
- **Test invariant violations.** For any size cap, write a test that exceeds it and asserts the result is capped.

### Database Method Delegation

- **The simple method must call the general method.** `save_message()` should delegate to `save_message_with_metadata(role, content, ct, None)` to prevent duplicate INSERT paths from diverging.

### Agent Loop Data Persistence

- **Enumerate all code paths that persist data.** The `EndTurn`, `max_steps_exceeded`, and timeout fallbacks must all go through the same persistence path.
- **Model data fully before storing it.** If a struct field is captured but never surfaced, the schema is incomplete.

## Files Modified

- `crates/mika-agent/src/agent.rs` — ToolCallSummary, process_tool_calls, run_loop, history builder, format_tool_summary_block
- `crates/mika-agent/src/db.rs` — ConversationMessage.metadata, save_message_with_metadata, SELECT queries
- `crates/mika-agent/src/async_db.rs` — Async wrapper for save_message_with_metadata
- `crates/mika-agent/src/compaction.rs` — extract_tool_names for compaction awareness

## References

- Plan: `docs/plans/2026-02-27-fix-tool-introspection-and-send-message-delivery-plan.md`
- Related: `docs/solutions/refactoring/agent-loop-variant-extraction-and-deduplication.md`
- Related: `docs/solutions/architecture/async-database-wrapper-pattern.md`
- Related: `docs/solutions/logic-errors/skill-availability-and-send-message-honesty.md`
- Branch: `fix/tool-introspection-and-send-message`
- Commits: `573596b` (initial), `420e99b` (review fixes)
