---
title: "fix: Parse cache token usage from OpenAI-compatible provider responses"
type: fix
status: completed
date: 2026-04-09
issue: 479
---

# fix: Parse cache token usage from OpenAI-compatible provider responses

## Overview

`OpenAiCompatibleProvider` silently drops cache token metrics from every LLM response. The `OpenAiUsage` struct only deserializes `prompt_tokens` and `completion_tokens`, discarding the standard `prompt_tokens_details.cached_tokens` field. The response mapping then hardcodes `cache_creation_input_tokens: None` and `cache_read_input_tokens: None`. This corrupts cost dashboards and cache-efficiency telemetry for all non-Anthropic providers (OpenRouter, OpenAI, Groq, Mistral, DeepSeek, etc.).

## Problem Statement

The downstream plumbing already works — `LlmUsage` has cache fields, the `llm_calls` table has `cache_read_tokens` and `cache_write_tokens` columns, and the DB insert code passes these values through. The only gap is the deserialization and mapping in `openai.rs`.

## Proposed Solution

A minimal, backward-compatible fix at a single code site.

### 1. Add `PromptTokensDetails` struct and extend `OpenAiUsage`

**File:** `crates/mika-common/src/llm/openai.rs` (lines 102-108)

```rust
#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    /// OpenAI-standard nested cache metrics.
    prompt_tokens_details: Option<PromptTokensDetails>,
}
```

**Design notes:**
- `Option<PromptTokensDetails>` handles providers that omit the field entirely (Groq, Ollama).
- `cached_tokens` uses `#[serde(default)]` so a `prompt_tokens_details` object without `cached_tokens` yields `0` (treated as no cache hit).
- `cache_creation_input_tokens` remains `None` — the OpenAI API spec does not expose write-through cache creation metrics (only Anthropic does).

### 2. Map cache fields in `from_openai_response()`

**File:** `crates/mika-common/src/llm/openai.rs` (lines 677-682)

```rust
let usage = resp.usage.map_or_else(LlmUsage::default, |u| {
    let cache_read = u
        .prompt_tokens_details
        .map(|d| d.cached_tokens)
        .filter(|&t| t > 0);
    LlmUsage {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: cache_read,
    }
});
```

**Design notes:**
- `.filter(|&t| t > 0)` converts `0` to `None` for consistency — no cache hit should be `None`, not `Some(0)`.
- `cache_creation_input_tokens` stays `None` — OpenAI-compatible providers don't report this.

### 3. Add info-level cache logging

**File:** `crates/mika-common/src/llm/openai.rs`, in `send_message()` after the API call

Add cache metric logging parity with the Anthropic provider (`claude.rs`):

```rust
if let (Some(read), _) = (
    response.usage.cache_read_input_tokens,
    response.usage.cache_creation_input_tokens,
) {
    tracing::info!(cache_read_tokens = read, "OpenAI-compatible provider cache metrics");
}
```

### 4. Unit tests

**File:** `crates/mika-common/src/llm/openai.rs` (test module)

Add tests covering these scenarios:

| Test | `prompt_tokens_details` | Expected `cache_read_input_tokens` |
|------|------------------------|------------------------------------|
| Cache hit (DeepSeek/OpenRouter) | `{"cached_tokens": 8192}` | `Some(8192)` |
| No cache details (Groq) | absent / `null` | `None` |
| Zero cached tokens | `{"cached_tokens": 0}` | `None` |
| Details present, field absent | `{}` (empty object) | `None` |

Each test constructs a full `OpenAiResponse` JSON payload and runs it through `from_openai_response()`, asserting on the `usage` fields of the resulting `LlmResponse`.

## Technical Considerations

- **No schema changes** — `llm_calls` table already has `cache_read_tokens` and `cache_write_tokens` columns.
- **No DB insert changes** — `agent.rs:636-637` already passes `resp.usage.cache_read_input_tokens` and `resp.usage.cache_creation_input_tokens`.
- **Backward compatible** — `Option` + `#[serde(default)]` means existing responses without cache fields parse identically to before (both `None`).
- **`completion_tokens_details`** — OpenAI also returns `completion_tokens_details.reasoning_tokens` for o1/o3 models. Out of scope for this fix (separate follow-up).
- **Flat cache fields** — Some providers (older DeepSeek API versions) may return flat `cache_read_input_tokens` directly on the usage object instead of nested. Current OpenRouter and OpenAI both use the nested format. If flat-field providers appear, a `#[serde(alias)]` or fallback can be added later — YAGNI for now.

## Acceptance Criteria

- [x] `OpenAiUsage` struct deserializes `prompt_tokens_details.cached_tokens` from OpenAI-compatible responses
- [x] `from_openai_response()` maps `cached_tokens` to `cache_read_input_tokens` on `LlmUsage`
- [x] Zero cached tokens maps to `None` (not `Some(0)`)
- [x] Absent `prompt_tokens_details` maps to `None` (backward compatible)
- [x] Info-level log emitted when cache read tokens are present
- [x] 4 unit tests covering: cache hit, no details, zero tokens, empty details object
- [x] `cargo test -p mika-common` passes
- [x] `cargo clippy` passes

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/llm/openai.rs` | Add `PromptTokensDetails` struct, extend `OpenAiUsage`, update `from_openai_response()` mapping, add cache logging, add 4 unit tests |

## Sources

- Related issue: #479
- Anthropic cache handling (reference): `crates/mika-common/src/claude.rs:257-262`
- Provider-agnostic type: `crates/mika-common/src/llm/types.rs:162-168`
- DB insert: `crates/mika-agent/src/agent.rs:636-637`
- Institutional learning: `docs/solutions/architecture-patterns/anthropic-prompt-caching-implementation.md`
