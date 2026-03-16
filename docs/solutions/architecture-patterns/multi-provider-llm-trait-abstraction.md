---
title: "Multi-Provider LLM Support via Trait Abstraction"
category: architecture-patterns
date: 2026-03-14
tags: [llm, trait-object, provider-pattern, openai, anthropic, rust]
related_issues: ["#71"]
---

# Multi-Provider LLM Support via Trait Abstraction

## Problem

Mika was tightly coupled to Anthropic's Claude API via `ClaudeClient`. Every consumer
(agent loop, compaction, team engine, task dispatcher, CLI, server) directly used
`ClaudeClient` and Anthropic-specific types (`Message`, `ContentBlock`, `MessagesRequest`).
Adding support for OpenAI, Ollama, Groq, or any OpenAI-compatible provider required
a clean abstraction without rewriting 37+ tool files or breaking existing functionality.

## Root Cause

No provider abstraction existed. The `ClaudeClient` was used directly throughout the
codebase, making it impossible to swap providers without touching every consumer.

## Solution

### 1. Provider-Agnostic Types (`mika-common/src/llm/types.rs`)

Created a parallel type system (`LlmRequest`, `LlmResponse`, `LlmMessage`, `LlmContent`,
`LlmContentBlock`, `LlmStopReason`, etc.) that is provider-neutral. `From` impls handle
bidirectional conversion between Anthropic types and agnostic types.

### 2. LlmProvider Trait (`mika-common/src/llm/mod.rs`)

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>;
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn max_tokens(&self) -> u32;
    fn supports_tool_calling(&self) -> bool { true }
    fn supports_vision(&self) -> bool { false }
    fn supports_extended_thinking(&self) -> bool { false }
    async fn check_health(&self) -> Result<(), LlmError>;
}
```

### 3. ModelSpec Parsing

`provider/model-name` format with backward-compatible default to Anthropic:
- `openai/gpt-4o` → OpenAI provider
- `ollama/llama3` → Ollama provider
- `claude-sonnet-4-6` → Anthropic (no prefix)

### 4. Key Design Decisions

- **Tool trait untouched**: `Tool` trait still returns `ToolDefinition` (Anthropic type).
  Conversion to `LlmToolDefinition` happens at `LlmRequest` construction via `.into()`.
  This avoided changing 37+ tool files.

- **Model switching**: Can't mutate `dyn LlmProvider`. Solution: recreate provider from
  settings when model changes via `/model` command.

- **Investigation panel**: Kept Anthropic-only (own mini agent loop). Creates fresh
  `ClaudeClient` since it's a specialized diagnostic feature.

- **`Arc<dyn LlmProvider>`**: All consumers hold `Arc<dyn LlmProvider>` instead of
  `ClaudeClient`. Factory method `Settings::make_llm_provider()` handles construction.

## Prevention

- When adding new LLM-dependent features, use `LlmProvider` trait, not `ClaudeClient` directly.
- The `investigate.rs` pattern (direct `ClaudeClient`) is the exception, not the rule.
- Test with `dummy_provider()` in unit tests — no API calls needed.
