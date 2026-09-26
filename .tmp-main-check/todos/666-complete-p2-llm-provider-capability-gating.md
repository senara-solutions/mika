---
status: pending
priority: p2
issue_id: "666"
tags: [code-review, architecture]
dependencies: []
---

# Gate Agent Loop Features on LLM Provider Capabilities

## Problem Statement

The `LlmProvider` trait has capability queries (`supports_tool_calling()`, `supports_vision()`, `supports_extended_thinking()`) but the agent loop does not check them before constructing requests with tools, images, or thinking config. If a user configures a provider that doesn't support function calling (e.g., some Ollama models), the provider will send tool definitions and get a confusing API error.

## Findings

- `LlmProvider` trait defines: `supports_tool_calling()`, `supports_vision()`, `supports_extended_thinking()`
- `OpenAiCompatibleProvider` returns `true` for `supports_tool_calling` but `false` for vision and thinking
- `agent.rs` unconditionally includes tools in `LlmRequest.tools` and images in messages
- No graceful degradation path exists

## Proposed Solutions

### Option 1: Check capabilities before request construction (Recommended)

In `run_agent_inner`, check `llm.supports_tool_calling()` before including tools. Check `llm.supports_vision()` before including images. Strip `thinking` if `!llm.supports_extended_thinking()`.

**Effort:** Medium.

## Acceptance Criteria

- [ ] `ollama/llama3` with `supports_tool_calling() = false` runs without tool definitions
- [ ] Images gracefully stripped with warning when vision unsupported
- [ ] ThinkingConfig ignored with warning when extended thinking unsupported
