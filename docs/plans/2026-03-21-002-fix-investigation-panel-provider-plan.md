---
title: "fix: Investigation panel hardcodes Anthropic provider instead of using configured LLM"
type: fix
status: completed
date: 2026-03-21
issue: 224
---

# fix: Investigation panel hardcodes Anthropic provider (#224)

## Problem

The investigation panel creates its own `ClaudeClient` directly instead of using the configured `dyn LlmProvider` from `AppState`. When a non-Anthropic provider is configured (e.g., `minimax/MiniMax-M2.5`), the investigation fails with "Claude API error: Authentication failed" because it tries to use the MiniMax API key as an Anthropic key.

Per the [multi-provider LLM trait abstraction](../solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md), `investigate.rs` is the **last holdout** using `ClaudeClient` directly.

## Proposed Solution

Refactor `crates/mika-agent/src/server/investigate.rs` to use provider-agnostic LLM types from `mika_common::llm` instead of Anthropic-specific types from `mika_common::claude`, and receive the provider from `AppState.llm` rather than constructing a new `ClaudeClient`.

## Acceptance Criteria

- [x] `investigate.rs` imports from `mika_common::llm`, not `mika_common::claude`
- [x] `run_investigation` takes `&dyn LlmProvider` instead of `&ClaudeClient`
- [x] `InvestigationParams.messages` uses `Vec<LlmMessage>` instead of `Vec<Message>`
- [x] Call site uses `state.llm.clone()` instead of `ClaudeClient::new()`
- [x] Investigation agent loop uses `LlmRequest`/`LlmResponse`/`LlmResponseContent`/`LlmStopReason`
- [x] Tool results use `LlmRole::Tool` with `LlmContentBlock::ToolResult` (`is_error: bool`, not `Option<bool>`)
- [x] Assistant message re-injection uses `response_content_to_blocks()`
- [x] Tool definitions converted via `.into()` (`ToolDefinition` → `LlmToolDefinition`)
- [x] Error messages changed from "Claude API error" to provider-agnostic text
- [x] Stale Anthropic-specific comments removed
- [x] `max_tokens: 4096` hardcoded in `LlmRequest` (preserves current behavior)
- [x] Model name from `llm.model_name().to_string()`
- [x] Investigation isolation preserved (separate lock, 5-step max, 120s timeout, read-only tools)
- [x] `cargo clippy` and `cargo test` pass
- [x] SSE event format unchanged (dashboard frontend compatibility)

## Implementation

### Type mapping

| Anthropic (`mika_common::claude`) | Provider-agnostic (`mika_common::llm`) |
|---|---|
| `ClaudeClient` | `dyn LlmProvider` (from `AppState.llm`) |
| `MessagesRequest` | `LlmRequest` |
| `Message` | `LlmMessage` |
| `MessageContent` | `LlmContent` |
| `ContentBlock` (response) | `LlmResponseContent` |
| `ContentBlock::ToolResult` | `LlmContentBlock::ToolResult` |
| `StopReason` | `LlmStopReason` |
| `ToolDefinition` | Keep (Tool trait returns it); convert via `.into()` at request build |
| `ToolResultBody` | `LlmToolResultContent` |

### Changes (single file: `investigate.rs`)

1. **Imports (~line 20-23):** Replace `mika_common::claude::*` with `mika_common::llm::*` types. Keep `ToolDefinition` from `claude` since the `Tool` trait returns it.

2. **`InvestigationParams` (~line 716):** `messages: Vec<Message>` → `messages: Vec<LlmMessage>`

3. **`run_investigation` signature (~line 724):** `claude: &ClaudeClient` → `llm: &dyn LlmProvider`

4. **Request building (~line 759-770):** `MessagesRequest { model, system, messages, tools, max_tokens }` → `LlmRequest { model: llm.model_name(), system, messages, tools: Some(defs.into_iter().map(|d| d.clone().into()).collect()), max_tokens: 4096, thinking: None }`

5. **API call (~line 778):** `claude.send_message(&request)` → `llm.send_message(&request)`

6. **Error message (~line 782):** `"Claude API error: {e}"` → `"LLM error: {e}"`

7. **Response parsing (~line 790-820):** Match `LlmResponseContent::Text` and `LlmResponseContent::ToolCall` instead of `ContentBlock::Text`/`ContentBlock::ToolUse`

8. **Stop reason matching:** `StopReason::EndTurn`/`StopReason::ToolUse` → `LlmStopReason::EndTurn`/`LlmStopReason::ToolUse`

9. **Tool result construction (~line 885):** `ContentBlock::ToolResult { is_error: if ... { Some(true) } else { None } }` → `LlmContentBlock::ToolResult { is_error: output.is_error }`

10. **History re-injection (~line 893-900):** Use `response_content_to_blocks()` for assistant messages; use `LlmRole::Tool` for tool result messages

11. **Message building at call site (~line 1040-1059):** Build `Vec<LlmMessage>` with `LlmRole::User`/`LlmRole::Assistant` and `LlmContent::Text`

12. **Provider construction (~line 1063-1079):** Remove `ClaudeClient::new()`, use `state.llm.clone()`, pass `llm.as_ref()` to `run_investigation`

13. **Remove stale comments** about Anthropic-specific types

### Design decisions

- **Hardcode `max_tokens: 4096`** in `LlmRequest` — preserves current investigation behavior regardless of server-wide provider config
- **Use `LlmRole::Tool`** for tool result messages — the translation layer maps this correctly per-provider (Anthropic → `"user"`, OpenAI → `"tool"`)
- **No `supports_tool_calling()` guard** — all current providers support tool calling; the trait default is `true`. If a future non-tool-calling provider is added, the main agent loop would also need similar guards (systemic concern, not investigation-specific)
- **No retry on LLM errors** — preserve current behavior (error → SSE error event → terminate)

## Sources

- Issue: [#224](https://github.com/senara-solutions/mika/issues/224)
- Learning: [multi-provider-llm-trait-abstraction](../solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md)
- Learning: [investigation-panel-sse-agent-loop](../solutions/architecture/investigation-panel-sse-agent-loop.md)
- Reference: `crates/mika-agent/src/agent.rs` (main agent loop — canonical `dyn LlmProvider` usage)
- Reference: `crates/mika-agent/src/server/mod.rs:353` (`state.llm` construction)
- Reference: `crates/mika-common/src/llm/types.rs` (type definitions and `From` impls)
