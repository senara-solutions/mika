---
title: "Fix Tool Summary LLM Mimicry — XML Context Tags"
date: 2026-02-28
category: logic-errors
severity: HIGH
module:
  - crates/mika-agent/src/agent.rs
tags:
  - tool-calls
  - llm-behavior
  - conversation-history
  - format
  - agent-loop
  - mimicry-prevention
symptoms:
  - "Claude generates fake [Tools used: ...] text instead of actual tool_use calls"
  - "After 3-5 conversational turns, tool calls stop working (stop_reason: EndTurn)"
  - "Handler scripts never invoked despite visible tool summaries in TUI"
related_files:
  - crates/mika-agent/src/agent.rs
  - CLAUDE.md
---

# Fix Tool Summary LLM Mimicry — XML Context Tags

## Problem Statement

After several conversation turns, Claude stops making actual `tool_use` API calls and instead generates text that mimics the tool summary format: `[Tools used: tmux_send_command(cargo test) -> Command sent]`. The TUI displays this text verbatim, making it appear that tools executed successfully. In reality, `stop_reason` is `EndTurn` (not `ToolUse`), and the tool handler is never invoked.

**Symptoms:**
- Tools work for the first few turns, then silently stop executing
- TUI shows `[Tools used: ...]` text in agent responses
- Handler script logs (e.g., `~/temp/tmux_commands.log`) show no entries for failing cases
- Problem worsens with conversation length (more historical examples to mimic)

## Root Cause

`format_tool_summary_block()` in `agent.rs:210` produced bracket-format summaries:

```
\n[Tools used: tmux_send_command(cargo test) -> Command sent; search_memory(q) -> found 3]
```

This format was appended to historical assistant messages when building the prompt for the Claude API (agent.rs:565). After several turns, Claude's conversation context contained multiple examples of this pattern in its own prior "responses."

**Why Claude mimics it:** Bracket notation with parenthesized arguments and arrow returns (`tool(input) -> output`) looks like functional notation — a pattern Claude can learn and reproduce. Claude memorized the format from its prior messages and generated matching text instead of emitting proper `tool_use` content blocks.

**Evidence chain:**
1. Tool summaries appended to assistant messages at `agent.rs:565`
2. Claude sees `[Tools used: ...]` in its conversation context
3. Claude generates text mimicking the pattern (`stop_reason: EndTurn`, no `tool_use` blocks)
4. TUI displays `output.text` verbatim at `chat.rs:129`
5. Handler script logs confirm the tool was never called

## Solution

Changed the tool summary format from bracket notation to XML `<context>` tags — the same pattern already used in 3 locations for conversation summaries and skill descriptions. Claude treats XML annotation tags as structured metadata and does not reproduce them in responses.

### Code Changes

**1. `crates/mika-agent/src/agent.rs:210` — format string:**

```rust
// Before (mimicable):
Some(format!("\n[Tools used: {}]", parts.join("; ")))

// After (non-mimicable):
Some(format!(
    "\n<context type=\"tool_history\" trust=\"metadata\">\n{}\n</context>",
    parts.join("\n")
))
```

**2. `crates/mika-agent/src/agent.rs:1519` — test assertion:**

```rust
// Before:
assert!(block.starts_with("\n[Tools used:"));

// After:
assert!(block.starts_with("\n<context type=\"tool_history\""));
assert!(block.contains("</context>"));
```

**3. `CLAUDE.md` — architecture documentation updated.**

### Design Decisions

- **`trust="metadata"`** — New trust level (existing: `"data"` for summaries, `"local"` for skills). Tool summaries are system-generated, distinct from user data or local skill definitions.
- **Newline-joined entries** — Each tool on its own line for readability (was semicolon-joined).
- **No data migration** — Metadata stored as JSON in `conversations.metadata` column; `format_tool_summary_block()` renders at load time. Old messages automatically render in new format.

### Existing `<context>` Tag Precedent

| Location | Type | Trust | Purpose |
|----------|------|-------|---------|
| agent.rs:541 | `summary` | `data` | Conversation summary in system prompt |
| agent.rs:946 | `summary` | `data` | Conversation summary in silent mode |
| agent.rs:1180 | `skill` | `local` | Skill descriptions in system prompt |
| agent.rs:210 | `tool_history` | `metadata` | Tool summaries in message history (NEW) |

## Prevention Strategies

### Why XML Tags Prevent Mimicry

XML `<context>` tags are structurally different from the bracket notation Claude mimicked:

- **Bracket notation** (`[Tools used: ...]`) resembles functional notation — arguments in parentheses, arrow returns, semicolon separators. Claude treats this as reproducible prose.
- **XML tags** (`<context type="...">`) are recognized as structural metadata annotations. Claude's training teaches it not to reproduce these in output text.

### Design Principles for LLM Context Injection

1. **Never use functional notation for metadata** — Brackets, parentheses, and arrows look like code Claude can reproduce. Use XML or plain narrative instead.
2. **Use trust attributes** — `trust="metadata"` signals immutability to Claude.
3. **Store metadata separately** — Persist in a database column, render at load time. Don't bake format into stored content.
4. **Single render path** — All messages go through `format_tool_summary_block()`. Format changes propagate everywhere.

### Detection Methods

- **Stop reason mismatch:** If `stop_reason == EndTurn` but response text contains tool-like patterns (`[Tools used:`, `→`, parenthesized arguments), mimicry is likely.
- **Handler log audit:** Cross-reference tool names in agent text responses against actual handler execution logs.
- **Database query:** Find assistant messages where content contains tool patterns but `metadata` column has no `tool_calls`.

## Related Documentation

- [Tool Call Introspection: Cross-Turn Persistence](./tool-call-introspection-cross-turn-persistence.md) — Establishes the metadata persistence layer that feeds into the format function
- [jq Pretty-Print Breaks Envelope Detection](./jq-pretty-print-envelope-detection.md) — Related pattern: format assumptions in protocol detection cause silent failures
- Plan: `docs/plans/2026-02-28-fix-tool-summary-llm-mimicry-plan.md`

## Follow-Up

- Consider updating `extract_tool_names()` in `compaction.rs:169` — uses bracket format `[used: tool1, tool2]` for summarization input. Lower risk (constrained single-turn call) but inconsistent.
- Empirical validation: Run 5-10 multi-turn conversations with tool usage, confirm Claude doesn't reproduce `<context>` tags.
