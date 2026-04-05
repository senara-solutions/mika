---
title: "Fix text-based tool calls: extract XML tool calls and add agent-loop recovery"
type: fix
status: active
date: 2026-04-05
---

# Fix text-based tool calls: extract XML tool calls and add agent-loop recovery

## Overview

Tool calling is broken across all providers using the OpenAI-compatible path. LLMs output tool calls as XML text (`<function=tool_name>...</function></tool_call>`) instead of structured API `tool_calls`. The `from_openai_response()` in `openai.rs` only extracts from the structured `choice.message.tool_calls` JSON field — XML in text content passes through as `LlmResponseContent::Text` and is displayed verbatim. Tools are never executed.

## Problem Statement

**Symptoms:**
- mika-dev outputs `<function=list_work_items>\n</function>\n</tool_call>` as visible text — the tool call is never executed
- The opening `<tool_call>` tag is missing from display while `</tool_call>` remains visible (likely the model not emitting it, not a stripping issue)
- Affects all non-Anthropic providers (all route through `OpenAiCompatibleProvider`)

**Investigation confirmed:**
- `strip_internal_tags()` does NOT strip `<tool_call>` — only 7 specific internal tags (`context`, `callback_result`, `task-health`, etc.)
- Tools ARE registered in `default_tools()` (~30 builtin tools) and serialized correctly in requests
- `supports_tool_calling()` returns `true` for all providers
- Both `to_openai_request()` and `to_anthropic_request()` correctly serialize tools
- No code teaches the LLM the `<function=...>` format — this is model behavior
- `from_openai_response()` only extracts tool calls from the structured `choice.message.tool_calls` JSON field

## Proposed Solution

Two-layer defense-in-depth fix:

### Layer 1: XML tool call extraction in `from_openai_response()` (primary fix)

Add `extract_xml_tool_calls()` in `crates/mika-common/src/llm/openai.rs` following the established `extract_think_block()` pattern. This parses XML-formatted tool calls from text content and converts them to `LlmResponseContent::ToolCall` items, making the agent loop's existing tool dispatch work without modification.

### Layer 2: Agent-loop detection and re-prompt (safety net)

Add `detect_text_based_tool_call()` in `crates/mika-agent/src/agent.rs`. In the `EndTurn` branch, detect remaining XML tool call patterns that Layer 1 missed, re-prompt the LLM to use structured tool calling. One retry allowed, following the existing `required_tools_retry_done` pattern.

### Diagnostic logging

Add `info!` log after `tools_for_request` is computed in the agent loop, logging tool count, provider, and model — confirms whether tools reach the API request.

## Technical Approach

### Phase 1: XML extraction function (`openai.rs`)

**New function: `extract_xml_tool_calls(text: &str) -> (Vec<LlmResponseContent>, String)`**

Returns extracted `ToolCall` items and remaining text (with XML removed).

**XML formats to handle (in priority order):**

1. **Wrapped function-tag:** `<tool_call>\n<function=func_name>\n{"key":"value"}\n</function>\n</tool_call>`
2. **Bare function-tag:** `<function=func_name>\n{"key":"value"}\n</function>` (no `<tool_call>` wrapper)
3. **JSON-in-tool_call:** `<tool_call>\n{"name":"func","arguments":{...}}\n</tool_call>`

**Regex strategy:** Use `LazyLock<Regex>` (consistent with `strip_internal_tags()`). Two regexes:
- `<tool_call>\s*(?:<function=([^>]+)>([\s\S]*?)</function>|(\{[\s\S]*?\}))\s*</tool_call>` — matches wrapped formats
- `<function=([^>]+)>([\s\S]*?)</function>` — matches bare function-tag (run after wrapped to avoid double-matching)

Both use lazy `[\s\S]*?` matching. Case-insensitive not needed (models produce consistent lowercase).

**Argument parsing:**
- If content is valid JSON object → use as `Value`
- If content is empty/whitespace → use `Value::Object(Map::new())` (empty `{}`)
- If content is invalid JSON → use `Value::String(content)` (same fallback as structured `tool_calls` at line 624)
- For JSON-in-tool_call format: parse `{"name":"...","arguments":{...}}` and extract fields

**Synthetic tool call IDs:** `xml_call_{n}` where n is 0-indexed within the response. Simple monotonic counter per response — sufficient since tool results are consumed before the next turn.

**Structured tool_calls precedence rule:** Skip XML extraction entirely when `choice.message.tool_calls` is `Some` and non-empty. This prevents duplicate tool calls when an LLM returns both formats.

### Phase 2: Integration into `from_openai_response()` (`openai.rs:580-662`)

Insert XML extraction AFTER text content extraction (line 606) and AFTER structured `tool_calls` extraction (line 627), but BEFORE `stop_reason` determination (line 629):

```
Current order:
  1. Extract text content (590-606)
  2. Extract structured tool_calls (609-627)
  3. Determine stop_reason from finish_reason (629-634)
  4. Extract <think> blocks (644-654)
  5. Remove empty text entries (656)

New order:
  1. Extract text content (590-606)
  2. Extract structured tool_calls (609-627)
  3. NEW: If no structured tool_calls, run extract_xml_tool_calls() on text content
     - Replace text items with stripped text (or remove if empty)
     - Append extracted ToolCall items to content
  4. Determine stop_reason — NEW: if content contains any ToolCall items AND
     finish_reason maps to EndTurn, override to ToolUse
  5. Extract <think> blocks (644-654)
  6. Remove empty text entries (656)
```

The `stop_reason` override is a post-processing fixup: check if `content` contains any `LlmResponseContent::ToolCall` items (from either structured or XML extraction) and `stop_reason` is `EndTurn` → flip to `ToolUse`. This also fixes a pre-existing edge case where some providers return structured `tool_calls` with `finish_reason: "stop"`.

Add `info!` log when XML extraction fires:
```rust
info!(
    extracted_count = tool_calls.len(),
    provider = "openai_compatible",
    "extracted XML-formatted tool calls from text response"
);
```

### Phase 3: Agent-loop recovery (`agent.rs`)

**New helper: `detect_text_based_tool_call(text: &str) -> bool`**

Simple pattern detection (no parsing — Layer 1 does the parsing):
- Check for `<function=` AND (`</function>` OR `</tool_call>`)
- Fast path: return `false` if text doesn't contain `<`

**Integration in `EndTurn` branch (agent.rs ~line 685):**

Add a new retry gate `text_tool_call_retry_done: bool` (initialized `false` before the loop, alongside `required_tools_retry_done`).

**Ordering with required_tools retry:** Text-based tool call retry fires FIRST (before required_tools check). Rationale: if the LLM outputs XML tool calls, re-prompting to use structured API is more likely to succeed than demanding specific tools be called.

```
EndTurn branch order:
  1. strip_internal_tags
  2. NEW: detect_text_based_tool_call → re-prompt (one retry)
  3. Existing: required_tools check → re-prompt (one retry)
  4. Existing: display text / handle empty
```

**Correction message:**
```
[Your response contained tool calls as text (e.g., <function=...>) instead of using
the structured tool calling API. Do NOT output tool calls as text. Use the tool
calling mechanism provided to you. Call the tool now using the proper API.]
```

Push the assistant's broken response as an assistant message first (so the model sees what it did wrong), then push the correction as a user message, then `continue`.

### Phase 4: Diagnostic logging (`agent.rs`)

Add `info!` log after `tools_for_request` is computed (~line 1241):

```rust
info!(
    tool_count = tools_for_request.as_ref().map_or(0, |t| t.len()),
    provider = llm.provider_name(),
    model = llm.model_name(),
    "preparing LLM request"
);
```

Add the same in `run_silent_agent()` and team agent loop for full coverage.

### Phase 5: Tests

**Unit tests in `openai.rs` (alongside existing tests at ~line 868):**

| Test | Description |
|------|-------------|
| `test_extract_xml_tool_calls_wrapped_function_tag` | `<tool_call><function=name>{...}</function></tool_call>` |
| `test_extract_xml_tool_calls_bare_function_tag` | `<function=name>{...}</function>` without wrapper |
| `test_extract_xml_tool_calls_json_in_tool_call` | `<tool_call>{"name":"...","arguments":{...}}</tool_call>` |
| `test_extract_xml_tool_calls_empty_arguments` | `<function=name></function>` → empty `{}` |
| `test_extract_xml_tool_calls_multiple` | Multiple tool calls in one response |
| `test_extract_xml_tool_calls_mixed_text` | Text before/after XML preserved, XML removed |
| `test_extract_xml_tool_calls_no_xml` | Plain text passthrough unchanged |
| `test_extract_xml_tool_calls_malformed` | Broken XML left as text |
| `test_extract_xml_tool_calls_with_think_block` | Think tags + tool call tags in same response |
| `test_from_openai_response_xml_tool_calls` | Full `from_openai_response()` integration with XML in text |
| `test_from_openai_response_xml_skipped_when_structured` | Structured tool_calls present → XML extraction skipped |
| `test_from_openai_response_stop_reason_flip` | `finish_reason: "stop"` with XML tool calls → `ToolUse` |

**Eval harness tests in `crates/mika-agent/tests/eval/test_tool_calling.rs`:**

| Test | Description |
|------|-------------|
| `test_text_based_tool_call_retry` | MockLlmProvider returns text with XML tool call pattern on first response, proper tool call on second → verify retry fires and tool executes |

**Unit test for `detect_text_based_tool_call()` in `agent.rs`:**

| Test | Description |
|------|-------------|
| `test_detect_text_based_tool_call_function_tag` | `<function=search>` detected |
| `test_detect_text_based_tool_call_tool_call_tag` | `</tool_call>` detected |
| `test_detect_text_based_tool_call_plain_text` | Normal text → not detected |
| `test_detect_text_based_tool_call_empty` | Empty string → not detected |

## System-Wide Impact

- **Interaction graph:** `from_openai_response()` → `extract_xml_tool_calls()` → `LlmResponseContent::ToolCall` → agent loop `ToolUse` branch → `process_tool_calls()` → tool execution → `tool_calls` table recording. Same path as structured tool calls — no new interactions.
- **Error propagation:** Malformed XML falls through as text (safe). Invalid JSON arguments fall back to `Value::String` (same as structured tool_calls). No new error paths.
- **State lifecycle risks:** None — tool calls are processed synchronously within a turn. No partial state.
- **API surface parity:** Only affects `OpenAiCompatibleProvider` response processing. Anthropic provider has separate `from_anthropic_response()` — not affected. The Anthropic `tool_use` format is already structured.
- **Observability:** XML-extracted tool calls flow through the same `process_tool_calls()` → `tool_calls` table path. `info!` log when extraction fires provides diagnostic visibility.

## Acceptance Criteria

- [x] XML tool calls in `<function=name>...</function>` format are extracted from text and executed as structured tool calls
- [x] XML tool calls wrapped in `<tool_call>...</tool_call>` are handled identically
- [x] JSON-in-tool_call format (`<tool_call>{"name":"...","arguments":{...}}</tool_call>`) is extracted
- [x] `stop_reason` is correctly set to `ToolUse` when XML tool calls are extracted
- [x] XML extraction is skipped when structured `tool_calls` are already present in the response
- [x] Remaining text (before/after XML) is preserved as `LlmResponseContent::Text`
- [x] Malformed XML is left as text (not extracted, not corrupted)
- [x] Agent loop detects text-based tool calls that Layer 1 missed, re-prompts once
- [x] Tool count diagnostic logging added to all agent loop entry points
- [x] All unit tests pass: `cargo test -p mika-common`
- [x] All eval tests pass: `cargo test -p mika-agent --test eval`
- [x] Full test suite passes: `cargo test`
- [x] No clippy warnings: `cargo clippy`

## Dependencies & Risks

- **Risk: False positives.** XML patterns in legitimate text content (e.g., user discussing XML syntax) could be incorrectly extracted. Mitigated by: only running extraction when structured `tool_calls` are absent, and requiring both opening and closing tags.
- **Risk: Provider-specific formats.** Different providers may emit slightly different XML formats. Mitigated by: handling the 3 known formats and using lazy matching.
- **Risk: Synthetic tool call ID format.** `xml_call_{n}` IDs may not be accepted by some providers in follow-up turns. Mitigated by: the IDs are only referenced in tool result messages which are consumed within the same turn.
- **No schema changes.** No DB migrations required.
- **No config changes.** This is a bug fix, not a feature flag.

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-common/src/llm/openai.rs` | Add `extract_xml_tool_calls()`, integrate into `from_openai_response()`, add unit tests |
| `crates/mika-agent/src/agent.rs` | Add `detect_text_based_tool_call()`, add retry in EndTurn branch, add diagnostic logging, add unit tests |
| `crates/mika-agent/tests/eval/test_tool_calling.rs` | Add eval test for text-based tool call retry |

## Sources

- Investigation 1: `~/.claude/plans/fancy-inventing-badger.md` — XML extraction approach
- Investigation 2: `~/.claude/plans/resilient-tickling-thompson.md` — agent-loop detection + re-prompt
- Related solution: `docs/solutions/2026-03-21-dispatch-relay-minimax-fixes.md` — `extract_think_block()` pattern origin
- Related solution: `docs/solutions/ui-bugs/strip-internal-metadata-tags-from-display.md` — `strip_internal_tags()` design
- Related solution: `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md` — retry pattern
- GitHub issue: #447
