# Multi-Provider LLM Support

**Date:** 2026-03-13
**Issue:** [#71](https://github.com/senara-solutions/mika/issues/71)
**Status:** Brainstorm

## What We're Building

An `LlmProvider` trait abstraction that decouples Mika's agent loop from Anthropic-specific types, enabling users to run Mika with alternative LLM providers — primarily local/self-hosted models (Ollama, vLLM) and cloud alternatives (OpenAI, Groq, Together).

### Motivation

- **Local/privacy models:** Users who can't send data to cloud APIs need local inference (Ollama, vLLM)
- **Flexibility & choice:** Teams standardized on OpenAI or other providers want to use Mika without an Anthropic key

### Non-Goals

- Automatic model routing or cost optimization (future work)
- Resilience/fallback between providers (future work)
- Gemini provider (can be added later using the same trait)

## Why This Approach

### Own Trait, No External Framework

Mika's no-framework philosophy extends to LLM abstraction. We'll build our own `LlmProvider` trait with per-provider adapters rather than adopting genai, rig, or other crates. Reasons:

- Full control over the abstraction surface — no fighting framework limitations
- Matches existing pattern (direct reqwest calls, no SDK dependencies)
- Avoids dependency churn from rapidly evolving LLM crates
- Can be exactly as thin or thick as Mika needs

### Claude-First, Others Best-Effort

Mika is designed for Claude. Other providers work but with documented limitations:
- Extended thinking: Claude-only, silently skipped on other providers
- Prompt caching: Claude-only
- Image content blocks: Translated where supported, skipped where not
- Tool calling quality varies by model — Mika's 10-step tool loop may not work well with weaker models

### Two Provider Formats Cover ~95% of Use Cases

1. **Anthropic format** — Claude models (existing implementation becomes `AnthropicProvider`)
2. **OpenAI-compatible format** — Covers OpenAI, Ollama, vLLM, Groq, Together, Fireworks, OpenRouter, LM Studio, DeepSeek, xAI

OpenAI-compatible is the de facto standard for local inference engines. Supporting it unlocks the entire local model ecosystem.

## Key Decisions

### 1. Provider-Agnostic Internal Types

Replace direct use of Anthropic-specific types (`ContentBlock`, `MessagesRequest`, `MessagesResponse`) in the agent loop with provider-agnostic types:

```rust
pub struct LlmRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<LlmMessage>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub max_tokens: u32,
    pub thinking: Option<ThinkingConfig>,  // Claude-only, ignored by others
}

pub struct LlmResponse {
    pub content: Vec<ResponseContent>,
    pub reasoning: Option<String>,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
}

pub enum ResponseContent {
    Text(String),
    ToolCall { id: String, name: String, arguments: Value },
}

pub enum StopReason { EndTurn, ToolUse, MaxTokens, ContentFilter }
```

### 2. LlmProvider Trait

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>;
    fn provider_name(&self) -> &str;
    fn supports_tool_calling(&self) -> bool { true }
    fn supports_vision(&self) -> bool { false }
    fn supports_extended_thinking(&self) -> bool { false }
}
```

### 3. Model String with Provider Prefix

Single config key with prefix-based provider routing:

```
MIKA_MODEL=anthropic/claude-sonnet-4-6    # Anthropic (default)
MIKA_MODEL=openai/gpt-4o                   # OpenAI
MIKA_MODEL=ollama/llama3                    # Ollama (local)
MIKA_MODEL=openai-compatible/deepseek-r1    # Generic OpenAI-compatible
```

For OpenAI-compatible providers, `MIKA_LLM_BASE_URL` overrides the endpoint:
```
MIKA_MODEL=openai-compatible/my-model
MIKA_LLM_BASE_URL=http://localhost:11434/v1
MIKA_LLM_API_KEY=...  # optional, some local providers don't need auth
```

Known prefixes auto-set base URLs:
- `ollama/` → `http://localhost:11434/v1`
- `groq/` → `https://api.groq.com/openai/v1`
- `openai/` → `https://api.openai.com/v1`

Backward compatibility: bare `MIKA_ANTHROPIC_API_KEY` + `MIKA_CLAUDE_MODEL` still works. If `MIKA_MODEL` is unset, falls back to existing config.

### 4. Tool Call Translation

The core translation challenge between Anthropic and OpenAI formats:

| Aspect | Anthropic | OpenAI |
|--------|-----------|--------|
| Tool def schema key | `input_schema` | `parameters` (in `function` wrapper) |
| Tool call location | Content block (`tool_use`) | Top-level `tool_calls` field |
| Arguments | `input` (JSON object) | `arguments` (JSON string!) |
| Tool result role | `user` + `tool_result` block | Dedicated `tool` role |
| System prompt | Top-level `system` field | `{"role": "system"}` message |

Each provider adapter handles this translation bidirectionally.

### 5. Migration Strategy

Refactor in layers to minimize blast radius:

1. **Extract provider-agnostic types** into `mika-common/src/llm/types.rs`
2. **Define `LlmProvider` trait** in `mika-common/src/llm/mod.rs`
3. **Wrap existing `ClaudeClient`** as `AnthropicProvider` implementing the trait
4. **Refactor agent loop** to use `Arc<dyn LlmProvider>` instead of `ClaudeClient`
5. **Add `OpenAiCompatibleProvider`** as second implementation
6. **Add config routing** based on model prefix
7. **Update all instantiation sites** (server, teams, task engine, delegate_task)

### 6. Where the Code Lives

- `crates/mika-common/src/llm/` — New module:
  - `mod.rs` — `LlmProvider` trait, factory function
  - `types.rs` — Provider-agnostic request/response types
  - `error.rs` — `LlmError` enum (replaces `ClaudeApiError` at the trait boundary)
  - `anthropic.rs` — Anthropic provider (wraps current `claude.rs` logic)
  - `openai.rs` — OpenAI-compatible provider
- `crates/mika-common/src/claude.rs` — Kept for Anthropic-specific wire types (serde structs), but no longer the public API

## Resolved Questions

1. **Compaction model:** Uses the main model (whatever provider the user configured). No separate compaction model pinning.

2. **Embedding provider:** Include in scope. Abstract `EmbeddingClient` alongside LLM providers — users running fully local (Ollama) want local embeddings too. Ollama exposes an OpenAI-compatible `/v1/embeddings` endpoint.

3. **Team mode:** Global provider only. All agents in a team use the same provider. Simplifies config and avoids cross-format edge cases in team runs.
