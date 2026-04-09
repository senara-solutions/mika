---
title: "OpenAiCompatibleProvider drops cache token usage from responses"
category: integration-issues
date: 2026-04-09
tags: [openai, openrouter, cache, telemetry, llm-provider, usage-tracking]
issue: 479
---

# OpenAiCompatibleProvider drops cache token usage from responses

## Problem

LLM calls routed through `OpenAiCompatibleProvider` (OpenRouter, OpenAI, Groq, Mistral, DeepSeek, etc.) always recorded `cache_read_tokens = 0` and `cache_write_tokens = 0` in the `llm_calls` telemetry table, even when upstream providers were actively serving cached prompts. This silently corrupted cost dashboards and cache-efficiency analysis for every non-Anthropic provider.

**Symptom:** Query `llm_calls` for any OpenAI-compatible provider — `cache_read_tokens` is always NULL/0 regardless of prompt size or session length.

## Root Cause

Two gaps in `crates/mika-common/src/llm/openai.rs`:

1. **Missing deserialization field.** `OpenAiUsage` struct only declared `prompt_tokens` and `completion_tokens`. The standard OpenAI response field `usage.prompt_tokens_details.cached_tokens` was silently discarded by serde (no matching struct field).

2. **Hardcoded `None` mapping.** `from_openai_response()` unconditionally set `cache_creation_input_tokens: None` and `cache_read_input_tokens: None` when constructing `LlmUsage`, ignoring any cache data even if it had been deserialized.

The downstream plumbing was already correct — `LlmUsage` had cache fields, the `llm_calls` DB table had the columns, and the insert code passed the values through. Only the response parsing was missing.

## Solution

**File:** `crates/mika-common/src/llm/openai.rs`

### 1. Added `PromptTokensDetails` struct

```rust
#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u64,
}
```

### 2. Extended `OpenAiUsage`

```rust
struct OpenAiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    prompt_tokens_details: Option<PromptTokensDetails>,  // NEW
}
```

### 3. Updated `from_openai_response()` mapping

```rust
let cache_read = u
    .prompt_tokens_details
    .map(|d| d.cached_tokens)
    .filter(|&t| t > 0);  // 0 → None for consistency
```

### 4. Added cache logging parity

Added `cache_read_tokens` to the existing info-level `llm_call completed` log in `send_message_inner()`, matching the Anthropic provider's logging.

**Key design decisions:**
- `Option<PromptTokensDetails>` handles providers that omit the field entirely (backward compatible).
- `cached_tokens` uses `#[serde(default)]` so an empty `{}` object yields `0`, which the `.filter(|&t| t > 0)` converts to `None`.
- `cache_creation_input_tokens` remains `None` — the OpenAI API spec does not expose write-through cache creation metrics (only Anthropic does).

## Prevention

- When adding new provider response types, always check the full API response schema (including nested `*_details` objects) rather than only the top-level fields.
- Compare the response struct against the provider-agnostic `LlmUsage` type — any `Option` field in `LlmUsage` that isn't mapped from the provider response is a potential data loss bug.
- The end-to-end JSON deserialization test pattern (constructing raw JSON → `serde_json::from_str` → `from_openai_response()`) catches schema mismatches that struct-level tests miss.
