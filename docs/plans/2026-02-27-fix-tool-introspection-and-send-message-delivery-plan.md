---
title: "Fix Tool Introspection and send_message Delivery"
type: fix
status: completed
date: 2026-02-27
---

# Fix Tool Introspection and send_message Delivery

## Overview

Two bugs identified from real usage of Mika with the claude-asked relay system:

1. **Tool execution introspection gap**: Mika cannot reference what tools it used in previous turns. When asked "what tmux command did you just send?", it cannot answer because tool call details (name, input, output) are only kept in memory during the current turn and never persisted to the `conversations` table.

2. **send_message delivery failure in relay context**: When the `claude-asked-relay` script calls `mika ask`, the `send_message` tool fails because `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN` are not available in the Claude Code hook environment. Mika reports "routing URL isn't configured" instead of forwarding dangerous commands to Vincent via Telegram.

## Problem Statement

### Issue 1: Tool Introspection

The agent loop in `crates/mika-agent/src/agent.rs` handles tool calls via `process_tool_calls()` (line 476). During a turn, tool call blocks (`ContentBlock::ToolUse` and `ContentBlock::ToolResult`) are accumulated in the in-memory `MessagesRequest.messages` vector. However, when the turn completes:

- Only the final text response is saved via `db.save_message("assistant", &text, ct)` (line 156)
- Tool call blocks are never persisted
- When `load_recent_messages(20, None)` rebuilds history for the next turn (line 377), it only produces `MessageContent::Text` messages
- All tool execution context is lost between turns

The `metadata TEXT` column exists in the `conversations` schema (line 178 of `db.rs`) but is never written to.

### Issue 2: send_message in Relay Context

The `claude-asked-relay` script (at `~/.local/bin/claude-asked-relay`) calls `mika ask "[claude-asked]..."`. This invokes the CLI one-shot path in `commands/ask.rs`, which calls `make_message_sender()` (line 18-19). This function returns `None` if either `MIKA_ROUTING_URL` or `MIKA_INTERNAL_TOKEN` is missing from the environment.

The relay script runs from Claude Code's hook context, which does not have Mika's server env vars. So `send_message` returns the warning: "No outbound sender configured — message was NOT delivered."

## Proposed Solution

### Fix 1: Persist Tool Call Summaries in Conversation Metadata

**Approach:** Modify `process_tool_calls` to return structured summaries, accumulate them across loop steps, and persist them in the `metadata` column when saving the assistant message.

**Data flow:**
1. `process_tool_calls` returns `Vec<ToolCallSummary>` alongside its current side effects
2. `run_loop` accumulates summaries across loop iterations
3. On `EndTurn`, save the assistant message with JSON metadata containing tool call summaries
4. `load_recent_messages` reads the metadata column
5. When building history, append a concise `[Tools used: ...]` block to assistant messages that had tool calls

**Metadata JSON schema:**
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

**Constraints:**
- Input summary: truncated to first 200 chars
- Output summary: truncated to first 300 chars
- Total metadata: capped at 4000 chars per message
- Applies to all `LoopMode` variants that save to DB (Conversation + Silent)

### Fix 2: Source Env Vars in Relay Script + Skill Prompt Update

**Two-pronged approach:**

1. **Update `claude-tmux-relay` skill system_prompt.md** to instruct Mika that when `send_message` fails (no outbound sender), it should fall back to logging the decision and auto-approving/denying based on its own judgment for safe commands, and for dangerous commands it should include a clear note in its response text that delivery failed.

2. **The relay script itself** (`~/.local/bin/claude-asked-relay`) is the user's script outside the Mika repo. Document that `MIKA_ROUTING_URL` and `MIKA_INTERNAL_TOKEN` should be exported in the shell environment where Claude Code runs, or sourced from a file in the relay script.

## Technical Considerations

### Database Layer Changes

- Add `metadata: Option<String>` to `ConversationMessage` struct
- Add `save_message_with_metadata()` method to `Database` (or extend `save_message`)
- Update `load_recent_messages` SELECT to include `metadata` column
- No schema migration needed — `metadata TEXT` column already exists at schema v4+

### Agent Loop Changes

- New `ToolCallSummary` struct: `{ step: u32, name: String, input_summary: String, output_summary: String, success: bool }`
- `process_tool_calls` signature changes to return `Vec<ToolCallSummary>`
- `run_loop` accumulates `Vec<ToolCallSummary>` across iterations
- New `save_assistant_response` helper that bundles text + metadata save
- History builder appends tool summary block to assistant messages when metadata is present

### Compaction Awareness

- Update compaction input to include tool names from metadata so summaries mention tool usage
- Only include tool names (not full details) to keep summarization prompts lean

### Files to Modify

**Core changes:**
- `crates/mika-agent/src/db.rs` — `ConversationMessage` struct, `save_message_with_metadata()`, `load_recent_messages` query
- `crates/mika-agent/src/agent.rs` — `ToolCallSummary` struct, `process_tool_calls` return type, `run_loop` accumulation, `run_agent_inner` save path, `run_silent_inner` save path, history building in `run_agent_inner`
- `crates/mika-agent/src/compaction.rs` — Include tool names in summarization input

**Skill/prompt changes:**
- `templates/skills/tmux/system_prompt.md` — No changes needed
- The user's `~/.mika/agents/main/skills/claude-tmux-relay/system_prompt.md` — Updated guidance for send_message failure fallback (documented, not auto-modified)

**Tests:**
- `crates/mika-agent/src/db.rs` — Tests for save/load with metadata
- `crates/mika-agent/src/agent.rs` — Tests for tool call summary accumulation, metadata serialization, history building with tool summaries

## Acceptance Criteria

### Fix 1: Tool Introspection
- [x] `process_tool_calls` returns `Vec<ToolCallSummary>` with name, truncated input/output, success flag
- [x] `run_loop` accumulates summaries and saves them as JSON in the `metadata` column
- [x] `load_recent_messages` loads the metadata column into `ConversationMessage`
- [x] History builder appends a concise tool summary block to assistant messages with tool calls
- [x] Tool summaries are truncated: 200 char input, 300 char output, 4000 char total metadata
- [x] Compaction includes tool names from metadata in summarization input
- [x] Existing tests pass, new tests cover save/load with metadata

### Fix 2: send_message Delivery
- [x] Mika can introspect its own tool calls when asked "what command did you just send?"
- [ ] Document env var requirements for the relay script context (user's custom script, outside repo)
- [ ] User updates their custom claude-tmux-relay skill prompt for send_message failure fallback

## Success Metrics

- Mika can answer "what did you just do?" or "what command did you send?" by referencing tool call metadata from the previous turn
- The claude-tmux-relay workflow functions correctly: safe commands auto-approved, dangerous commands forwarded or clearly flagged

## Dependencies & Risks

- **No schema migration needed** — the metadata column already exists
- **Token budget impact** — Tool summaries in history add ~50-100 tokens per assistant message that used tools. For 20 messages loaded, worst case ~2000 extra tokens. Acceptable.
- **Privacy** — Tool outputs may contain sensitive data. Truncation mitigates but doesn't eliminate. Acceptable for plaintext-on-encrypted-volume model.
- **Backward compatibility** — Older messages with `NULL` metadata handled gracefully (no tool summary block appended)

## References

- Agent loop: `crates/mika-agent/src/agent.rs:476` (process_tool_calls)
- DB schema: `crates/mika-agent/src/db.rs:173` (conversations table)
- send_message tool: `crates/mika-agent/src/tools/send_message.rs:63`
- CLI ask command: `crates/mika-cli/src/commands/ask.rs:12`
- make_message_sender: `crates/mika-cli/src/init.rs:129`
- Compaction: `crates/mika-agent/src/compaction.rs`
- Past learning: `docs/solutions/logic-errors/skill-availability-and-send-message-honesty.md`
- Past learning: `docs/solutions/integration-issues/cli-telegram-messaging-and-skill-seeding.md`
