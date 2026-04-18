---
title: "XML tool calls displayed as text instead of being executed"
category: runtime-errors
date: 2026-04-05
tags: [tool-calling, openai-compatible, xml-parsing, llm-response, agent-loop]
issue: "#447"
modules: [mika-common::llm::openai, mika-agent::agent]
---

# XML tool calls displayed as text instead of being executed

## Problem

Tool calling broken across all OpenAI-compatible providers. LLMs output tool calls as XML text (`<function=tool_name>...</function></tool_call>`) instead of structured API `tool_calls`. The text is displayed verbatim — tools are never executed.

**Symptoms:**
- mika-dev outputs `<function=list_tasks>\n</function>\n</tool_call>` as visible text
- Opening `<tool_call>` tag missing from display while `</tool_call>` remains (model behavior, not a stripping issue)
- Affects all non-Anthropic providers (all route through `OpenAiCompatibleProvider`)

## Root Cause

`from_openai_response()` in `openai.rs` only extracts tool calls from the structured `choice.message.tool_calls` JSON field. When a model outputs tool calls as XML text in `choice.message.content`, they pass through as `LlmResponseContent::Text`. There was no XML tool call extraction — it had never been implemented.

Investigation confirmed: tools ARE registered and serialized correctly in requests; `supports_tool_calling()` returns `true`; both request serializers correctly include tools. The problem is purely in response parsing.

## Solution

Two-layer defense-in-depth fix:

**Layer 1 — Response processing (`mika-common::llm::openai`):**
- Added `extract_xml_tool_calls()` following the established `extract_think_block()` pattern
- Handles 3 XML formats: wrapped `<tool_call><function=...>`, bare `<function=...>`, and JSON-in-tool_call
- Uses `LazyLock<Regex>` with lazy `[\s\S]*?` matching (Rust regex crate guarantees linear-time, no ReDoS)
- Integrated into `from_openai_response()` — runs only when no structured `tool_calls` present
- Flips `stop_reason` from `EndTurn` to `ToolUse` when tool calls are extracted
- Generates synthetic IDs (`xml_call_{n}`) for extracted tool calls

**Layer 2 — Agent loop recovery (`mika-agent::agent`):**
- Added `detect_text_based_tool_call()` — lightweight pattern check for `<function=` + closing tags
- In EndTurn branch, fires before required_tools check — re-prompts LLM once to use structured API
- Follows the existing `required_tools_retry_done` single-retry pattern

**Diagnostic logging:**
- Added `info!` log with tool count, provider, model at all agent loop entry points (conversation, silent, team)

## Key Design Decisions

1. **Structured tool_calls take precedence** — XML extraction is skipped entirely when `choice.message.tool_calls` is present. Prevents duplicate tool calls.
2. **Two-pass regex** — wrapped patterns extracted first, then bare `<function=...>` patterns. Prevents double-matching.
3. **Same dispatch path** — extracted tool calls produce `LlmResponseContent::ToolCall` items identical to structured ones. No changes needed in the agent loop's tool dispatch.
4. **Argument fallback** — invalid JSON arguments fall back to `Value::String` (same as structured tool_calls path).

## Prevention

- When adding new LLM response processing, always consider that providers may embed structured content in text using XML-like tags (same pattern as `<think>` blocks).
- The `extract_think_block()` → `extract_xml_tool_calls()` pattern can be reused for future tag extraction needs.
- Use `MIKA_LOG_LLM_BODIES=true` to inspect raw provider responses when debugging tool calling issues.

## Related

- `docs/solutions/2026-03-21-dispatch-relay-minimax-fixes.md` — introduced `extract_think_block()` pattern
- `docs/solutions/ui-bugs/strip-internal-metadata-tags-from-display.md` — `strip_internal_tags()` design
- `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md` — retry pattern reused for Layer 2
