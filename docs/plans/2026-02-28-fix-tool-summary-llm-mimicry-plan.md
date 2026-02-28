---
title: "Fix tool summary LLM mimicry"
type: fix
status: completed
date: 2026-02-28
---

# Fix: Tool Summary LLM Mimicry

## Overview

Claude mimics the `[Tools used: ...]` text pattern in its own responses instead of making actual `tool_use` calls. After several conversational turns, Claude sees the bracket-format tool summaries appended to its historical assistant messages and learns to reproduce them verbatim as text output — with `stop_reason: EndTurn` instead of `ToolUse`. The tool handler is never invoked, but the user sees fake tool execution summaries in the TUI.

## Problem Statement

**Root cause:** `format_tool_summary_block()` (`agent.rs:210`) produces `\n[Tools used: tmux_send_command(cargo test) → Command sent]` — a pattern that looks like functional notation. Claude memorizes this from its own prior "responses" and generates matching text instead of emitting proper `tool_use` content blocks.

**Evidence chain:**
1. Tool summaries are appended to historical assistant messages at `agent.rs:565`
2. Claude sees `[Tools used: ...]` in its conversation context
3. Claude generates text mimicking the pattern (EndTurn, no tool_use blocks)
4. TUI displays `output.text` verbatim at `chat.rs:129`
5. Handler script logs confirm the tool was never called

## Proposed Solution

Wrap tool summaries in XML `<context>` tags — the same pattern already used in 3 locations for conversation summaries and skill descriptions. Claude treats XML annotation tags as structured metadata and does not reproduce them in responses.

**Format change:**
```
# Before (mimicable)
\n[Tools used: tmux_send_command(cargo test) → Command sent; search_memory(q) → found 3]

# After (non-mimicable)
\n<context type="tool_history" trust="metadata">
tmux_send_command(cargo test) → Command sent
search_memory(q) → found 3
</context>
```

## Technical Considerations

- **No data migration needed:** The `metadata` column stores raw JSON; `format_tool_summary_block` renders at load time. Old messages will automatically render in the new format.
- **Existing precedent:** `<context type="summary" trust="data">` (lines 541, 946) and `<context type="skill" trust="local">` (line 1180) already use this pattern, but in the system prompt. This fix places `<context>` tags in assistant message text — a different structural position that should be validated empirically.
- **Compaction format:** `extract_tool_names()` in `compaction.rs:169` produces `" [used: tool1, tool2]"` in bracket format for summarization input. Lower risk since it's a constrained single-turn call, but tracked as follow-up for consistency.
- **Trust attribute:** `trust="metadata"` is a new value (existing: `"data"`, `"local"`). Tool summaries are system-generated, distinct from user data or local skill definitions.

## Acceptance Criteria

- [x] `format_tool_summary_block()` outputs `<context type="tool_history" trust="metadata">` XML format
- [x] Each tool entry on its own line (newline-joined, not semicolon-joined)
- [x] Test `test_format_tool_summary_block_valid_json` updated to assert XML format
- [x] New assertion: output contains closing `</context>` tag
- [x] All existing tests pass (`cargo test`)
- [x] `cargo clippy` clean
- [x] CLAUDE.md updated: "History builder appends `<context type=\"tool_history\">` blocks" (replaces `[Tools used: ...]` reference)

## MVP

### `crates/mika-agent/src/agent.rs` — `format_tool_summary_block()` (line 210)

```rust
// Before:
Some(format!("\n[Tools used: {}]", parts.join("; ")))

// After:
Some(format!(
    "\n<context type=\"tool_history\" trust=\"metadata\">\n{}\n</context>",
    parts.join("\n")
))
```

### `crates/mika-agent/src/agent.rs` — test (line 1519)

```rust
// Before:
assert!(block.starts_with("\n[Tools used:"));

// After:
assert!(block.starts_with("\n<context type=\"tool_history\""));
assert!(block.contains("</context>"));
```

### `CLAUDE.md` — Architecture section

Update the line referencing `[Tools used: ...]` to reflect the new XML format.

## Follow-Up (Out of Scope)

- Consider updating `extract_tool_names()` in `compaction.rs:169` to use XML format for consistency
- Update solution doc `docs/solutions/logic-errors/tool-call-introspection-cross-turn-persistence.md`
- Manual validation: run 5-10 conversational turns with tool calls, confirm Claude doesn't reproduce `<context>` tags

## References

- Problem description: `~/temp/compiled-jumping-aho.md`
- Current code: `crates/mika-agent/src/agent.rs:170-211` (format_tool_summary_block)
- History injection: `crates/mika-agent/src/agent.rs:554-578`
- Existing `<context>` pattern: `agent.rs:541`, `agent.rs:946`, `agent.rs:1180`
- Related solution doc: `docs/solutions/logic-errors/tool-call-introspection-cross-turn-persistence.md`
