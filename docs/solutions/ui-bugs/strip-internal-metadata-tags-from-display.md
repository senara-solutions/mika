---
title: Strip internal metadata tags from LLM response display
category: ui-bugs
date: 2026-03-27
tags: [agent-loop, llm-response, tag-stripping, tui, display, regex]
issue: "#223"
related_pr: "#221"
---

# Strip internal metadata tags from LLM response display

## Problem

Internal XML metadata tags like `<context type="tool_history" trust="metadata">...</context>` were displayed as raw text in the TUI, `mika ask`, and server outbound messages (Telegram). The LLM echoes these tags because they are injected into conversation history by the history builder (`format_tool_summary_block()` in `agent.rs`).

**Symptom:** Users see raw XML blocks in chat responses:
```
<context type="tool_history" trust="metadata">
send_message({"text":"claude-pilot is running mika-cloud #15..."})
[TOOL_RESULT] → Message sent.
</context>
```

**Affected tags:** `<context>`, `<callback_result>`, `<task-health>` (with nested `<active-work-items>`, `<anomalies>`, `<task-health-instructions>`), `<rewind_reversals>`.

## Root Cause

No tag-stripping existed anywhere in the response pipeline. The flow `run_loop()` → `LoopResult.text` → `AgentOutput.text` → display/persistence passed raw LLM text unmodified. The LLM sees these tags in reconstructed conversation history and occasionally echoes them verbatim.

## Solution

### 1. `strip_internal_tags()` function (`mika-common::llm`)

Per-tag lazy-compiled regexes (not backreferences — Rust `regex` crate doesn't support them). Each tag gets a pattern `(?s)<{tag}\b[^>]*>.*?</{tag}>` with dotall mode and lazy matching.

**Key design choices:**
- **Early-exit fast path:** `if !text.contains('<')` skips regex entirely (common case — most responses have no tags)
- **Per-tag regexes in `LazyLock<Vec<Regex>>`** — compiled once, reused across calls
- **Blank-line collapse:** `\n{3,}` → `\n\n` after tag removal to avoid visual gaps
- **Trim result:** Leading/trailing whitespace removed; empty result signals callers to convert to `None`
- **`<think>` excluded:** Handled separately by `extract_think_block()` in `openai.rs` for OpenAI-compatible providers (extracts thinking content, not just strips)

### 2. Application points

| Location | Purpose |
|----------|---------|
| `run_loop()` EndTurn text extraction (line 623) | All conversation, silent, and team normal responses |
| `run_agent_inner()` continuation response (line 1259) | Max-steps-exceeded continuation turn |
| `run_team_agent_inner_impl()` continuation response (line 2185) | Team agent max-steps continuation |
| `send_message` tool (line 52) | Proactive outbound messages (bypasses AgentOutput) |

### 3. System prompt defense-in-depth

Added instruction in `prompt.rs`: "Never include internal XML tags like `<context>`, `<callback_result>`, `<task-health>`, or `<rewind_reversals>` in your responses."

## Key Gotcha: Rust regex crate has no backreferences

The plan originally proposed a single regex with `\1` backreference:
```rust
r"(?s)<(context|callback_result|...)\b[^>]*>.*?</\1>"
```

This **fails at runtime** with `error: backreferences are not supported`. The solution is per-tag compiled regexes iterated in a loop. The performance difference is negligible (microseconds vs seconds of LLM API latency).

## Prevention

- When adding new internal XML tags to system prompts or conversation history, add the tag name to `INTERNAL_TAG_NAMES` in `mika-common/src/llm/mod.rs`
- The `strip_internal_tags()` function is called on every LLM response — new tags are automatically stripped once added to the list
- Consider whether any new injection point also needs stripping in the `send_message` tool path
