---
title: "fix: enable prompt caching for agent LLM calls"
type: fix
status: completed
date: 2026-03-28
issue: "#302"
---

# fix: enable prompt caching for agent LLM calls

## Overview

Agent LLM calls show 0% cache efficiency — `cache_read_tokens = 0` across all calls despite repeated context (system prompts, skill prompts, conversation history). The Anthropic client reads cache metrics from responses but never sends `cache_control` breakpoints in requests.

From task audit of mika#299: 49 LLM calls, 1.5M input tokens, **0% cache efficiency**. 40 delegate sessions each re-sent ~30K tokens of nearly identical system prompt. Prompt caching would reduce cost by 60-80% on the Anthropic path.

## Problem Statement

1. `MessagesRequest.system` is `Option<String>` — a plain string. Anthropic's caching requires `system` to be an array of content blocks with `cache_control: {"type": "ephemeral"}` annotations.
2. `Usage` correctly deserializes `cache_creation_input_tokens` and `cache_read_input_tokens` from responses — but they're always 0 because caching was never requested.
3. No `cache_control` code exists anywhere in the codebase.

## Proposed Solution

Handle all `cache_control` injection in the **Anthropic translation layer** (`to_anthropic_request()` in `anthropic.rs`), keeping provider-agnostic types (`LlmRequest`, `LlmMessage`) unchanged. This is Anthropic-specific; other providers are unaffected.

**Two cache breakpoints (initial implementation):**
1. **System prompt** — convert `system: Option<String>` into `system: Option<Vec<SystemContentBlock>>` with `cache_control: {"type": "ephemeral"}` on the last block. Caches the entire system prompt prefix (soul, identity, instructions, skills, core memory).
2. **Tool definitions** — add `cache_control: {"type": "ephemeral"}` to the last `ToolDefinition`. Caches tool schemas (stable across the conversation).

**Deferred:** Message-level caching (last user message `cache_control`) — requires modifying `ContentBlock` enum variants and provides less incremental benefit since messages change every turn. Follow-up ticket.

## Technical Approach

### New Types in `claude.rs`

```rust
#[derive(Debug, Serialize, Clone)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub kind: String, // "ephemeral"
}

impl CacheControl {
    pub fn ephemeral() -> Self {
        Self { kind: "ephemeral".to_string() }
    }
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename = "text")]
pub struct SystemContentBlock {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}
```

### Changes to `MessagesRequest` in `claude.rs`

```rust
// Before:
pub system: Option<String>,

// After:
#[serde(skip_serializing_if = "Option::is_none")]
pub system: Option<Vec<SystemContentBlock>>,
```

### Changes to `ToolDefinition` in `claude.rs`

```rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}
```

### Translation in `anthropic.rs` — `to_anthropic_request()`

```rust
// Convert system string into cached content block array
let system = req.system.as_ref().map(|s| {
    vec![SystemContentBlock {
        text: s.clone(),
        cache_control: Some(CacheControl::ephemeral()),
    }]
});

// Add cache_control to last tool definition
let mut tools: Vec<ToolDefinition> = /* existing translation */;
if let Some(last) = tools.last_mut() {
    last.cache_control = Some(CacheControl::ephemeral());
}
```

### Update `check_health()` in `anthropic.rs`

The health check constructs `MessagesRequest` directly — update to use the new system type:
```rust
system: None, // was already None, no type change needed for None
```

### Update tests in `claude.rs`

Three test functions construct `MessagesRequest` with `system: Some("...".into())` — update to use `Some(vec![SystemContentBlock { text: "...".into(), cache_control: None }])`.

### Add cache metrics to info logging in `claude.rs`

Add cache hit/write token counts to the existing success log line in `send_message_inner()`:
```rust
info!(
    model, input_tokens, output_tokens,
    cache_read_tokens = usage.cache_read_input_tokens.unwrap_or(0),
    cache_write_tokens = usage.cache_creation_input_tokens.unwrap_or(0),
    "API response"
);
```

## System-Wide Impact

- **Interaction graph**: `to_anthropic_request()` is the sole entry point. All agent modes (conversation, silent, team, delegate) pass through `AnthropicProvider::send_message()` → `to_anthropic_request()` → `ClaudeClient::send_message()`. No callbacks or observers fire.
- **Error propagation**: If the Anthropic API rejects the new system format (it shouldn't — content block arrays are the documented format), `ClaudeApiError` handles it via existing HTTP status retry logic.
- **State lifecycle risks**: None. This is a stateless request-building change — no persistence, no partial failure states.
- **API surface parity**: Only `AnthropicProvider` is affected. `OpenAiCompatibleProvider` path is untouched. OpenRouter users with Anthropic models go through `OpenAiCompatibleProvider` and will NOT get prompt caching (documented limitation).
- **Integration test scenarios**: (1) Multi-turn conversation verifying cache_read_tokens > 0 on turn 2+. (2) Skill LLM override to non-Anthropic verifying no cache_control in request. (3) Health check still works with new system type.

## Acceptance Criteria

- [x] `MessagesRequest.system` serializes as `[{"type": "text", "text": "...", "cache_control": {"type": "ephemeral"}}]` — `crates/mika-common/src/claude.rs`
- [x] Last `ToolDefinition` has `cache_control: {"type": "ephemeral"}` in serialized output — `crates/mika-common/src/claude.rs`
- [x] `to_anthropic_request()` converts `LlmRequest.system: Option<String>` to cached content blocks — `crates/mika-common/src/llm/anthropic.rs`
- [x] `check_health()` compiles and works with new system type — `crates/mika-common/src/llm/anthropic.rs`
- [x] Existing tests updated and passing — `crates/mika-common/src/claude.rs`
- [x] New serialization test verifying cache_control JSON format — `crates/mika-common/src/claude.rs`
- [x] Cache metrics (read/write tokens) logged at info level — `crates/mika-common/src/claude.rs`
- [x] Provider-agnostic types (`LlmRequest`, `LlmMessage`, `LlmContentBlock`) remain unchanged — `crates/mika-common/src/llm/types.rs`
- [x] `cargo test` passes, `cargo clippy` clean
- [x] Non-Anthropic providers unaffected (no behavioral change)

## Success Metrics

- Cache read tokens > 0 on multi-turn Anthropic conversations (verified in `llm_calls` table or dashboard)
- 60-80% cache hit rate for typical agent sessions with stable system prompts
- No regressions on non-Anthropic provider paths

## Dependencies & Risks

- **Risk: Anthropic API format change** — Low. Content block system format is documented and GA. No beta header required.
- **Risk: 4-breakpoint limit** — Using 2 of 4 allowed breakpoints (system + tools). Well within limits.
- **Risk: Minimum token threshold** — Anthropic requires ≥2,048 tokens for caching. Mika's system prompt is typically 5K-30K tokens. Only edge case: test/health requests with tiny/no system prompt — these have `system: None`, so no cache_control is sent.
- **Known limitation:** OpenRouter Anthropic models go through `OpenAiCompatibleProvider` and won't get prompt caching.

## MVP

### crates/mika-common/src/claude.rs

1. Add `CacheControl` struct with `ephemeral()` constructor
2. Add `SystemContentBlock` struct with `text` and optional `cache_control`
3. Change `MessagesRequest.system` from `Option<String>` to `Option<Vec<SystemContentBlock>>`
4. Add `cache_control: Option<CacheControl>` to `ToolDefinition`
5. Add cache metrics to success log line in `send_message_inner()`
6. Update 3 test functions that construct `MessagesRequest` directly
7. Add new test for cache_control serialization

### crates/mika-common/src/llm/anthropic.rs

1. Update `to_anthropic_request()` to convert `system: Option<String>` → `Option<Vec<SystemContentBlock>>` with `cache_control: ephemeral` on the last block
2. Add `cache_control: ephemeral` to last tool definition after building the tools vec
3. Update `check_health()` if it constructs `MessagesRequest` directly (verify `system: None` still works)

## Sources

- Related issue: #302
- Anthropic prompt caching docs: https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching
- Architecture pattern: `docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md`
- Observability infra: `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md`
