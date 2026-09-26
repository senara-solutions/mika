---
title: "feat: Add multi-provider LLM support"
type: feat
status: active
date: 2026-03-13
origin: docs/brainstorms/2026-03-13-multi-provider-llm-brainstorm.md
---

# feat: Add multi-provider LLM support

## Overview

Introduce an `LlmProvider` trait abstraction that decouples Mika's agent loop from Anthropic-specific types, enabling users to run Mika with OpenAI-compatible LLM providers (OpenAI, Ollama, vLLM, Groq, Together, etc.) alongside the existing Anthropic provider. Also abstract the embedding client to support local embedding providers (Ollama).

**Issue:** [#71](https://github.com/senara-solutions/mika/issues/71)

## Problem Statement

Mika is hard-coupled to Anthropic's Claude API. The `ClaudeClient` struct, `MessagesRequest`, `MessagesResponse`, `ContentBlock`, and `StopReason` types are used directly in the agent loop, compaction, investigation panel, team engine, and task dispatcher across 10+ files. Users who need local inference (privacy, cost, air-gapped environments) or prefer other providers (OpenAI, Groq) cannot use Mika without an Anthropic API key.

## Proposed Solution

Provider-agnostic internal types + per-provider adapters (see brainstorm: `docs/brainstorms/2026-03-13-multi-provider-llm-brainstorm.md`). Own trait, no external framework. Claude-first, others best-effort.

## Technical Approach

### Architecture

```
┌─────────────────────────────────────────────────────┐
│                    Agent Loop                        │
│  (works with LlmRequest/LlmResponse/StopReason)     │
│                                                      │
│  agent.rs, compaction.rs, investigate.rs, teams/     │
└──────────────────────┬──────────────────────────────┘
                       │ Arc<dyn LlmProvider>
┌──────────────────────▼──────────────────────────────┐
│              LlmProvider Trait                        │
│  send_message(&LlmRequest) -> Result<LlmResponse>   │
│  provider_name() -> &str                             │
│  supports_tool_calling() -> bool                     │
│  supports_vision() -> bool                           │
│  supports_extended_thinking() -> bool                │
│  check_health() -> Result<()>                        │
└───────────┬──────────────────────┬──────────────────┘
            │                      │
  ┌─────────▼────────┐  ┌─────────▼──────────────────┐
  │ AnthropicProvider │  │ OpenAiCompatibleProvider    │
  │                   │  │                             │
  │ Wraps existing    │  │ Covers: OpenAI, Ollama,    │
  │ claude.rs logic   │  │ vLLM, Groq, Together,      │
  │                   │  │ Fireworks, DeepSeek, etc.   │
  └───────────────────┘  └─────────────────────────────┘
```

Similarly for embeddings:

```
┌──────────────────────────────────────┐
│         EmbeddingProvider Trait       │
│  embed(&[String]) -> Vec<Vec<f32>>   │
│  dimensions() -> usize               │
└──────────┬───────────────┬──────────┘
           │               │
  ┌────────▼──────┐  ┌────▼──────────────────┐
  │ OpenAiEmbed   │  │ OpenAiCompatibleEmbed  │
  │ (current)     │  │ (Ollama /v1/embeddings)│
  └───────────────┘  └───────────────────────┘
```

### Implementation Phases

#### Phase 1: Provider-Agnostic Types and Trait Definition

Extract provider-agnostic types into a new `llm` module in `mika-common`. Define the `LlmProvider` trait.

**New files:**

- `crates/mika-common/src/llm/mod.rs` — Module root, `LlmProvider` trait, `create_provider()` factory
- `crates/mika-common/src/llm/types.rs` — Provider-agnostic request/response types
- `crates/mika-common/src/llm/error.rs` — `LlmError` enum

**Types to define in `types.rs`:**

```rust
// crates/mika-common/src/llm/types.rs

pub struct LlmRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<LlmMessage>,
    pub tools: Option<Vec<LlmToolDefinition>>,
    pub max_tokens: u32,
    pub thinking: Option<ThinkingConfig>,  // Provider-specific, ignored if unsupported
}

pub struct LlmMessage {
    pub role: LlmRole,
    pub content: LlmContent,
}

pub enum LlmRole {
    User,
    Assistant,
    Tool,  // OpenAI "tool" role; mapped to user+tool_result for Anthropic
}

pub enum LlmContent {
    Text(String),
    Blocks(Vec<LlmContentBlock>),
}

pub enum LlmContentBlock {
    Text(String),
    Image(LlmImage),
    ToolCall { id: String, name: String, arguments: Value },
    ToolResult { tool_call_id: String, content: LlmToolResultContent, is_error: bool },
}

pub enum LlmToolResultContent {
    Text(String),
    Blocks(Vec<LlmToolResultBlock>),
}

pub enum LlmToolResultBlock {
    Text(String),
    Image(LlmImage),
}

pub struct LlmImage {
    pub media_type: String,
    pub data: String,  // base64-encoded
}

pub struct LlmToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,  // JSON Schema
}

pub struct LlmResponse {
    pub content: Vec<LlmResponseContent>,
    pub reasoning: Option<String>,
    pub stop_reason: LlmStopReason,
    pub usage: LlmUsage,
}

pub enum LlmResponseContent {
    Text(String),
    ToolCall { id: String, name: String, arguments: Value },
}

pub enum LlmStopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    ContentFilter,
}

pub struct LlmUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: Option<u64>,  // Anthropic-only
    pub cache_read_input_tokens: Option<u64>,       // Anthropic-only
}

pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
```

**Trait in `mod.rs`:**

```rust
// crates/mika-common/src/llm/mod.rs

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

**Error in `error.rs`:**

```rust
// crates/mika-common/src/llm/error.rs

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("HTTP error (status {status}): {message}")]
    HttpError { status: u16, message: String, retryable: bool },
    #[error("Transport error: {0}")]
    Transport(String),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Provider error: {0}")]
    ProviderError(String),
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
}
```

**Tasks:**
- [x] Create `crates/mika-common/src/llm/mod.rs` with trait + `create_provider()` factory
- [x] Create `crates/mika-common/src/llm/types.rs` with all provider-agnostic types
- [x] Create `crates/mika-common/src/llm/error.rs` with `LlmError`
- [x] Add `pub mod llm;` to `crates/mika-common/src/lib.rs`
- [x] Keep existing `ToolDefinition` in `claude.rs` as Anthropic wire type, add `LlmToolDefinition` as the agnostic version
- [x] Implement `From<ToolDefinition> for LlmToolDefinition` and vice versa (they're structurally similar)
- [x] Add tests for type conversions

#### Phase 2: AnthropicProvider Implementation

Wrap existing `ClaudeClient` logic into an `AnthropicProvider` that implements `LlmProvider`.

**New file:** `crates/mika-common/src/llm/anthropic.rs`

The `AnthropicProvider` translates between `LlmRequest`/`LlmResponse` and Anthropic-specific wire types (`MessagesRequest`/`MessagesResponse`/`ContentBlock`). The existing `claude.rs` is preserved as-is for Anthropic wire type definitions and HTTP transport.

**Translation responsibilities:**
- `LlmRequest` → `MessagesRequest`: Map `LlmMessage` to `Message`, `LlmContentBlock` to `ContentBlock`, `LlmToolDefinition` to `ToolDefinition`
- `MessagesResponse` → `LlmResponse`: Map `ContentBlock` to `LlmResponseContent`, `StopReason` to `LlmStopReason`, `Usage` to `LlmUsage`
- `LlmRole::Tool` + `ToolResult` blocks → Anthropic's `user` role with `ContentBlock::ToolResult`
- `ThinkingConfig` passed through (Anthropic supports it)
- `ImageSource` → `LlmImage` and back

**Tasks:**
- [x] Create `crates/mika-common/src/llm/anthropic.rs`
- [x] Implement `LlmProvider` for `AnthropicProvider`
- [x] Implement bidirectional type conversions (LlmRequest ↔ MessagesRequest, LlmResponse ↔ MessagesResponse)
- [x] Wire `check_health()` to existing minimal API call pattern from doctor
- [x] Set capability flags: `supports_tool_calling: true`, `supports_vision: true`, `supports_extended_thinking: true`
- [x] Add unit tests for all translation paths (text, tool calls, tool results, images, thinking blocks)

#### Phase 3: OpenAI-Compatible Provider Implementation

**New file:** `crates/mika-common/src/llm/openai.rs`

Implements the OpenAI Chat Completions API format, usable with OpenAI, Ollama, vLLM, Groq, Together, etc.

**Key translation differences from Anthropic:**
- System prompt → first message with `role: "system"` (not top-level field)
- Tool definitions → wrapped in `{"type": "function", "function": {"name", "description", "parameters"}}` (not flat `{name, description, input_schema}`)
- Tool call arguments → JSON string (not parsed object) — must `serde_json::from_str()` on response
- Tool call response location → top-level `tool_calls` array on assistant message (not content blocks)
- Tool results → `role: "tool"` messages with `tool_call_id` (not `user` role with `tool_result` block)
- Images → `image_url` content with `data:` URI (not `image` block with base64 source)
- Stop reason → `stop`/`tool_calls`/`length`/`content_filter` (different strings)
- Usage → `prompt_tokens`/`completion_tokens` (different field names)
- No thinking/extended reasoning support (silently skip)

**OpenAI wire types (serde structs for serialization):**

```rust
// crates/mika-common/src/llm/openai.rs

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    max_tokens: u32,
}

#[derive(Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,  // "system", "user", "assistant", "tool"
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OpenAiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

// ... etc. (full OpenAI Chat Completions API types)
```

**Tasks:**
- [x] Create `crates/mika-common/src/llm/openai.rs` with OpenAI wire types
- [x] Implement `OpenAiCompatibleProvider` struct with configurable `base_url` and optional `api_key`
- [x] Implement `LlmProvider` trait for `OpenAiCompatibleProvider`
- [x] Implement `LlmRequest` → `OpenAiRequest` translation (system prompt as message, tool format wrapping, image format conversion)
- [x] Implement `OpenAiResponse` → `LlmResponse` translation (tool_calls extraction, arguments JSON string parsing, stop reason mapping, usage field mapping)
- [x] Handle malformed tool call arguments gracefully (log warning, return error to agent)
- [x] Set capability flags: `supports_tool_calling: true` (configurable), `supports_vision: false` (configurable), `supports_extended_thinking: false`
- [x] Wire `check_health()` to `GET /v1/models` or a minimal completion call
- [x] Add unit tests for all translation paths including edge cases (malformed args, missing usage, no tool support)

#### Phase 4: Model String Parsing and Provider Factory

**Modify:** `crates/mika-common/src/llm/mod.rs` — add `create_provider()` factory

**Model string format:** `provider/model-name` (see brainstorm: key decision #3)

```rust
// crates/mika-common/src/llm/mod.rs

pub struct ModelSpec {
    pub provider: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Ollama,
    Groq,
    OpenAiCompatible,
}

impl ModelSpec {
    pub fn parse(model_string: &str) -> Result<Self> { ... }
}

pub fn create_provider(spec: &ModelSpec) -> Result<Arc<dyn LlmProvider>> { ... }
```

**Known prefix → default base URL mapping:**
- `anthropic/` → Anthropic API (existing URL)
- `openai/` → `https://api.openai.com/v1`
- `ollama/` → `http://localhost:11434/v1`
- `groq/` → `https://api.groq.com/openai/v1`
- `openai-compatible/` → requires `MIKA_LLM_BASE_URL`
- No prefix → defaults to `anthropic/` (backward compatibility)

**API key resolution order:**
1. `MIKA_LLM_API_KEY` (generic override)
2. Provider-specific: `MIKA_ANTHROPIC_API_KEY`, `MIKA_OPENAI_API_KEY`, `MIKA_GROQ_API_KEY`
3. For `ollama/`: no key required (default)

**Tasks:**
- [x] Implement `ModelSpec::parse()` with prefix extraction and validation
- [x] Implement `create_provider()` factory function
- [x] Handle unknown prefixes: error with list of known prefixes
- [x] Handle models with slashes in name (e.g., `ollama/meta-llama/llama-3.1-8b` — only first slash is the separator)
- [x] Add unit tests for prefix parsing edge cases

#### Phase 5: Configuration and Settings Updates

**Modify:** `crates/mika-common/src/config.rs`

**New config keys:**
- `model` (env: `MIKA_MODEL`) — replaces `claude_model`, format: `provider/model-name`
- `llm_base_url` (env: `MIKA_LLM_BASE_URL`) — override for OpenAI-compatible providers
- `llm_api_key` (env: `MIKA_LLM_API_KEY`) — generic API key (falls back to provider-specific)
- `embedding_model` (env: `MIKA_EMBEDDING_MODEL`) — replaces hardcoded `text-embedding-3-small`, format: `provider/model-name`
- `embedding_base_url` (env: `MIKA_EMBEDDING_BASE_URL`) — override for embedding endpoint

**Backward compatibility:**
- `MIKA_CLAUDE_MODEL` → treated as `anthropic/{value}` if `MIKA_MODEL` is unset
- `MIKA_ANTHROPIC_API_KEY` → still used as the API key when provider is `anthropic`
- `MIKA_CLAUDE_MAX_TOKENS` → still used for max_tokens
- `MIKA_OPENAI_API_KEY` → used for `openai/` provider AND for embeddings (existing behavior preserved)

**Priority cascade:** `MIKA_MODEL` > `MIKA_CLAUDE_MODEL` (with provider prefix added). Log deprecation warning when only `MIKA_CLAUDE_MODEL` is set.

**Follow config key checklist** (from learnings: `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`):
- [x] Add `ConfigKeyInfo` entries for new keys
- [x] Add fields to `Settings` struct with serde defaults
- [x] Add `get_effective_value()` match arms
- [x] Add secret redaction in manual `Debug` impl for `llm_api_key`
- [x] Update `.env.example` with new keys
- [x] Update `docs/configuration.md`
- [x] Update handler script env scrubbing for new `MIKA_*` keys
- [x] Add `mika setup` support for provider selection
- [x] Add `mika doctor` checks for new keys

#### Phase 6: Refactor Agent Loop and Consumers

**Modify:** All files that currently use `ClaudeClient` directly.

**Agent loop (`crates/mika-agent/src/agent.rs`):**
- Replace `claude: &ClaudeClient` parameter with `llm: &Arc<dyn LlmProvider>` in all entry points (`run_agent`, `run_silent_agent`, `run_team_agent`)
- Replace `MessagesRequest` construction with `LlmRequest` construction
- Replace `ContentBlock` pattern matching with `LlmResponseContent` matching
- Replace `StopReason` matching with `LlmStopReason` matching
- Replace `Message`/`MessageContent` with `LlmMessage`/`LlmContent`
- Replace `ImageSource` with `LlmImage` in `AgentParams.user_images`
- Update `strip_prior_images()` to work on `LlmContentBlock::Image`
- Update `process_tool_calls()` to extract `LlmResponseContent::ToolCall`
- Update `ThinkingConfig` handling: only pass through if `llm.supports_extended_thinking()`
- Update `AgentOutput.usage` to use `LlmUsage`

**Other consumers:**
- [x] `crates/mika-agent/src/compaction.rs` — use `Arc<dyn LlmProvider>` instead of `ClaudeClient`
- [x] `crates/mika-agent/src/server/investigate.rs` — use `Arc<dyn LlmProvider>`, update tool def and response handling
- [x] `crates/mika-agent/src/server/state.rs` — `AppState` holds `Arc<dyn LlmProvider>` instead of `ClaudeClient`
- [x] `crates/mika-agent/src/server/mod.rs` — construct provider via factory, pass to state
- [x] `crates/mika-agent/src/teams/engine.rs` — `EngineResources` holds `Arc<dyn LlmProvider>`
- [x] `crates/mika-agent/src/task_engine/dispatcher.rs` — use provider from state
- [x] `crates/mika-agent/src/task_engine/engine.rs` — use provider
- [x] `crates/mika-agent/src/tools/delegate_task.rs` — construct provider for delegated agent
- [x] `crates/mika-cli/src/init.rs` — construct provider via factory
- [x] `crates/mika-cli/src/tui/app.rs` — replace `ClaudeClient::dummy()` with a no-op provider
- [x] `crates/mika-cli/src/commands/chat.rs` — update `ImageSource` → `LlmImage`, `ThinkingConfig` usage
- [x] `crates/mika-cli/src/commands/doctor.rs` — use provider's `check_health()`
- [x] `crates/mika-agent/src/server/handlers.rs` — update `ImageSource` conversion

**Tool system (minimal changes):**
- [x] Update `ToolDefinition` imports: the existing `ToolDefinition` in `claude.rs` stays as Anthropic wire type. The `Tool` trait's `fn definition(&self) -> ToolDefinition` continues to return the existing type (provider-agnostic in structure — `name`, `description`, `input_schema` as JSON Value). The provider adapter converts this to wire format. **No changes needed to 37 tool files.**
- [x] Alternatively, if we rename `ToolDefinition` to `LlmToolDefinition` and re-export, tool files update their import path but no code changes.

#### Phase 7: Embedding Provider Abstraction

**New files:**
- `crates/mika-common/src/llm/embedding.rs` — `EmbeddingProvider` trait + factory
- `crates/mika-common/src/llm/openai_embedding.rs` — Current `EmbeddingClient` logic, implements trait

**Trait:**

```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, LlmError>;
    fn dimensions(&self) -> usize;
    fn provider_name(&self) -> &str;
}
```

**Implementations:**
- `OpenAiEmbeddingProvider` — wraps current `EmbeddingClient` logic (works for OpenAI and Ollama's `/v1/embeddings`)
- No separate Ollama implementation needed — Ollama's OpenAI-compatible `/v1/embeddings` endpoint works with `OpenAiEmbeddingProvider` + custom base URL

**Re-indexing on dimension change:**
- On startup, check if stored embedding dimensions match current provider's dimensions
- If mismatch: drop all embeddings from `sqlite-vec`, trigger background re-index from fact text
- Log clear warning: "Embedding dimensions changed (was 512, now 768). Re-indexing all facts..."
- FTS5 full-text search continues to work during re-indexing (graceful degradation)

**Tasks:**
- [ ] Create `EmbeddingProvider` trait
- [ ] Implement `OpenAiEmbeddingProvider` (wrapping current `EmbeddingClient`)
- [ ] Add `create_embedding_provider()` factory using `MIKA_EMBEDDING_MODEL` prefix parsing
- [ ] Add dimension-change detection and re-indexing logic
- [ ] Update all `EmbeddingClient` usage sites to use `Arc<dyn EmbeddingProvider>`
- [x] Add tests

#### Phase 8: Doctor and Setup Updates

**Modify:** `crates/mika-cli/src/commands/doctor.rs`, `crates/mika-cli/src/commands/setup.rs`

**Doctor changes:**
- Provider-aware API key validation (Anthropic format check, or skip for Ollama)
- Provider-aware live check: `llm.check_health()` instead of hardcoded Anthropic call
- Display active provider and model in doctor output
- Warn if `thinking_level` is set but provider doesn't support it
- Warn if `MIKA_CLAUDE_MODEL` is used (deprecation)
- Check embedding provider connectivity separately

**Setup changes:**
- `mika setup` offers provider selection (Anthropic default, OpenAI, Ollama, custom)
- Prompts for appropriate API key based on provider
- For Ollama: checks if server is running, offers to pull model

**Tasks:**
- [x] Update `check_api_key` to dispatch by provider
- [x] Update `check_api_live` to use `check_health()`
- [x] Add provider/model display to doctor output
- [x] Add capability warnings (thinking_level, vision)
- [x] Update setup wizard for provider selection
- [x] Add deprecation warning for `MIKA_CLAUDE_MODEL`

## Alternative Approaches Considered

1. **Use genai crate** — Rejected: adds external dependency, conflicts with no-framework philosophy, may not support all Mika-specific needs (see brainstorm: "Own Trait, No External Framework")
2. **Use rig crate** — Rejected: too framework-like, associated types complicate dynamic dispatch
3. **Use Anthropic types as canonical internal representation** — Considered but rejected: leaky abstraction, OpenAI provider would need to parse to/from Anthropic-shaped types which is confusing. Provider-agnostic types are cleaner.
4. **LiteLLM proxy as external dependency** — Not rejected but deferred: can be used as an `openai-compatible` target for users who want it. No code needed in Mika.

## System-Wide Impact

### Interaction Graph

Config change → `Settings` loads model string → `ModelSpec::parse()` extracts provider → `create_provider()` instantiates correct `LlmProvider` → `Arc<dyn LlmProvider>` injected into agent loop, compaction, investigation, teams, task dispatcher → each consumer calls `send_message()` with `LlmRequest` → provider translates to wire format → HTTP call → provider translates response → consumer processes `LlmResponse`.

### Error & Failure Propagation

`LlmError` replaces `ClaudeApiError` at the trait boundary. `LlmError::HttpError { retryable }` replaces status-code-based retry logic. Each provider determines retryability: Anthropic (429/500/529), OpenAI (429/500/503). `anyhow::Result` wrapping continues in application code.

### State Lifecycle Risks

**Embedding dimension mismatch:** Switching embedding providers can orphan stored vectors. Mitigated by dimension-change detection and re-indexing on startup.

**Config migration:** Users with existing `MIKA_CLAUDE_MODEL` in `.env` or `config.toml` must not break. Backward compatibility layer handles this.

### API Surface Parity

All internal APIs that expose `ClaudeClient` must change to `Arc<dyn LlmProvider>`: `AgentParams`, `SilentAgentParams`, `TeamAgentParams`, `AppState`, `EngineResources`. External HTTP API is unchanged.

### Integration Test Scenarios

1. **Full agent loop with Anthropic provider:** User sends message → agent calls tools → responds. Validates end-to-end with existing behavior.
2. **Full agent loop with OpenAI-compatible provider (mock server):** Same flow but using OpenAI format. Validates tool call translation round-trip.
3. **Backward compatibility:** Existing `.env` with only `MIKA_ANTHROPIC_API_KEY` + `MIKA_CLAUDE_MODEL` works without changes.
4. **Embedding re-indexing:** Change embedding model dimensions → verify re-index triggers and search still works.
5. **Doctor with non-Anthropic provider:** `mika doctor` reports correct provider, runs appropriate health check.

## Acceptance Criteria

### Functional Requirements

- [x] `MIKA_MODEL=anthropic/claude-sonnet-4-6` works identically to current behavior
- [x] `MIKA_MODEL=openai/gpt-4o` with `MIKA_OPENAI_API_KEY` runs agent loop with tool calls
- [x] `MIKA_MODEL=ollama/llama3` runs agent loop against local Ollama (requires tool-capable model)
- [x] `MIKA_MODEL=openai-compatible/model` with `MIKA_LLM_BASE_URL` works for custom endpoints
- [x] Existing config (`MIKA_CLAUDE_MODEL` + `MIKA_ANTHROPIC_API_KEY`) continues to work
- [x] `MIKA_EMBEDDING_MODEL=ollama/nomic-embed-text` with `MIKA_EMBEDDING_BASE_URL` uses local embeddings
- [x] Extended thinking silently skipped for non-Anthropic providers
- [x] Image attachments translated for OpenAI-compatible providers
- [x] `mika doctor` validates configured provider
- [x] Tool calls work correctly with OpenAI format (arguments JSON string parsing, role mapping)

### Non-Functional Requirements

- [x] No performance regression for Anthropic provider (zero-cost abstraction where possible)
- [x] All new API keys redacted in logs and Debug impl
- [x] New env vars scrubbed from child processes (exec handler, MCP)

### Quality Gates

- [x] All existing tests pass (no regression)
- [x] Unit tests for all type translation paths (Anthropic ↔ agnostic, OpenAI ↔ agnostic)
- [x] Unit tests for model string parsing edge cases
- [x] Integration test with mock OpenAI-compatible server
- [x] `cargo clippy` clean
- [x] Documentation updated (configuration.md, CLAUDE.md, .env.example)

## Dependencies & Prerequisites

- None — all work is internal refactoring + new code
- Optional: Ollama installed locally for manual testing of local provider flow

## Risk Analysis & Mitigation

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Agent loop regression from type changes | Medium | High | Incremental refactoring, existing test suite, keep claude.rs wire types intact |
| Tool call translation bugs with OpenAI | Medium | High | Comprehensive unit tests for every translation path, test with real OpenAI API |
| Backward compatibility break | Low | High | Priority cascade logic, existing config continues to work, deprecation warnings |
| Local models with poor tool calling | High | Medium | Clear error messages, `mika doctor` warnings, documentation of recommended models |
| Embedding re-index data loss | Low | Medium | FTS5 fallback ensures search continues working during re-index |

## Future Considerations

- **Gemini provider:** Third format, can be added as another `LlmProvider` implementation
- **Streaming support:** The trait could add `fn stream_message()` in the future
- **Model routing:** Different models for different tasks (cheap for compaction, powerful for agent)
- **Fallback chains:** Try provider A, fall back to provider B on failure
- **Per-agent model selection in teams:** Currently global, could be per-agent
- **Provider-specific options passthrough:** Temperature, top_p, etc. via config

## Documentation Plan

- [x] Update `docs/configuration.md` — new env vars, model string format, provider list
- [x] Update `CLAUDE.md` — new module structure, new env vars
- [x] Update `.env.example` — add `MIKA_MODEL`, `MIKA_LLM_BASE_URL`, `MIKA_LLM_API_KEY`, `MIKA_EMBEDDING_MODEL`
- [x] Add `docs/adr/NNN-multi-provider-llm-abstraction.md` — ADR for the trait design decision
- [x] Update `docs/getting-started.md` — mention provider options

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-13-multi-provider-llm-brainstorm.md](docs/brainstorms/2026-03-13-multi-provider-llm-brainstorm.md) — Key decisions carried forward: own trait (no framework), Claude-first best-effort, model prefix routing, OpenAI-compatible covers ~95% of providers

### Internal References

- Current Claude client: `crates/mika-common/src/claude.rs`
- Current embedding client: `crates/mika-common/src/embedding.rs`
- Agent loop: `crates/mika-agent/src/agent.rs`
- Config system: `crates/mika-common/src/config.rs`
- Config key rename checklist: `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`
- Config 4-source model: `docs/solutions/architecture-patterns/simplified-config-4-source-model.md`
- MCP integration patterns: `docs/solutions/integration-issues/mcp-client-integration-rmcp.md`

### External References

- OpenAI Chat Completions API: https://platform.openai.com/docs/api-reference/chat
- Ollama OpenAI compatibility: https://docs.ollama.com/api/openai-compatibility
- Rust genai crate (pattern reference): https://crates.io/crates/genai
- Industry research on multi-provider patterns: documented in brainstorm
