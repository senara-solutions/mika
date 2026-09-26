---
title: "Anthropic Prompt Caching Implementation"
category: architecture-patterns
date: 2026-03-28
tags: [llm, anthropic, prompt-caching, cost-optimization, cache-control]
related_issues: ["#302"]
---

# Anthropic Prompt Caching Implementation

## Problem

Agent LLM calls showed 0% cache efficiency — `cache_read_tokens = 0` across all calls despite repeated context. Observed: 49 LLM calls, 1.5M input tokens, 0% cache hits. 40 delegate sessions each re-sent ~30K tokens of nearly identical system prompt. The Anthropic client deserialized cache metrics from responses but never sent `cache_control` breakpoints in requests.

## Root Cause

`MessagesRequest.system` was `Option<String>` — a plain string. Anthropic's prompt caching requires `system` to be an array of content blocks with `cache_control: {"type": "ephemeral"}` annotations. No `cache_control` code existed anywhere in the codebase.

## Solution

All `cache_control` injection happens in the **Anthropic translation layer** (`to_anthropic_request()` in `anthropic.rs`), keeping provider-agnostic types unchanged.

### New Types (claude.rs)

```rust
// Zero-alloc enum with serde tag
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CacheControl {
    #[serde(rename = "ephemeral")]
    Ephemeral,
}

// System content block with cache_control, type tag hardcoded via serde
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename = "text")]
pub struct SystemContentBlock {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

// Wrapper to add cache_control to tools without modifying 50+ Tool impls
#[derive(Debug, Clone, Serialize)]
pub struct CachedToolDefinition {
    #[serde(flatten)]
    pub tool: ToolDefinition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}
```

### Key Design Decision: CachedToolDefinition Wrapper

`ToolDefinition` is returned by the `Tool` trait's `definition()` method, used in 50+ tool files. Adding `cache_control` directly would require touching all of them. The `CachedToolDefinition` wrapper uses `#[serde(flatten)]` to produce identical JSON while only touching 2 files:

- `claude.rs` — new types, `MessagesRequest.tools` type change
- `anthropic.rs` — translation layer wraps tools and injects cache_control

### Translation (anthropic.rs)

```rust
fn to_anthropic_request(req: &LlmRequest) -> MessagesRequest {
    // System: single cached content block
    let system = req.system.as_ref()
        .map(|s| vec![SystemContentBlock::text_cached(s.clone())]);

    // Tools: wrap in CachedToolDefinition, cache_control on last only
    let tools = req.tools.as_ref().map(|tools| {
        let mut defs: Vec<CachedToolDefinition> = tools.iter()
            .map(|t| CachedToolDefinition {
                tool: ToolDefinition { /* ... */ },
                cache_control: None,
            })
            .collect();
        if let Some(last) = defs.last_mut() {
            last.cache_control = Some(CacheControl::ephemeral());
        }
        defs
    });
    // ...
}
```

### Cache Breakpoint Strategy

Two of four allowed Anthropic breakpoints used:
1. **System prompt** — caches the entire system prompt prefix (soul, identity, instructions, skills, core memory)
2. **Last tool definition** — caches tool schemas (stable across conversation)

Message-level caching (last user message) deferred — requires modifying `ContentBlock` variants and provides less benefit since messages change every turn.

## Prevention

- **When adding provider-specific API features**: Handle them in the provider translation layer (`to_anthropic_request` / `to_openai_request`), not in provider-agnostic types. This keeps `LlmRequest`/`LlmMessage` clean.
- **When adding fields to widely-used structs**: Use wrapper types with `#[serde(flatten)]` to avoid touching dozens of implementing files. The wrapper pattern keeps the original struct stable.
- **Verify cache efficiency**: Check `llm_calls` table `cache_read_tokens` and `cache_write_tokens` columns after deployment. Dashboard LLM Calls page shows these metrics.

## Known Limitations

- OpenRouter Anthropic models go through `OpenAiCompatibleProvider` and do not benefit from prompt caching
- Minimum 2,048 tokens required for Anthropic caching to activate (Mika's system prompt is typically 5K-30K tokens, well above threshold)
