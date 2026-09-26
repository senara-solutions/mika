---
title: "MODEL_ALIASES bare Anthropic names break on OpenRouter"
category: ui-bugs
date: 2026-04-06
tags: [provider, model, alias, openrouter, tui, cli]
issues: []
---

# MODEL_ALIASES bare Anthropic names break on OpenRouter

## Problem

After switching to OpenRouter via `/provider openrouter`, using `/model sonnet` sets the model to `claude-sonnet-4-6` (bare name) instead of `anthropic/claude-sonnet-4-6` (valid OpenRouter model name). The `/model` command hint also shows `[sonnet|opus|haiku]` which is Anthropic-specific.

## Root Cause

`MODEL_ALIASES` in `cli.rs` had inconsistent formatting:
- 3 Anthropic aliases used bare model names: `("sonnet", "claude-sonnet-4-6", ...)`
- 3 cross-provider aliases included provider prefix: `("gpt4o", "openai/gpt-4o", ...)`

On OpenRouter, `parse_provider_model()` returns the full input as the model name (because `model_names_contain_slash()` is true). Bare names like `claude-sonnet-4-6` are not valid OpenRouter model IDs — they require the `anthropic/` prefix.

The cross-provider aliases (`gpt4o`, `deepseek`, `gemini`) worked on OpenRouter by accident because their `full_id` already contained a slash (`openai/gpt-4o`), which happens to be a valid OpenRouter model name.

## Solution

**Normalize all `MODEL_ALIASES` to include provider prefix:**

```rust
pub const MODEL_ALIASES: &[(&str, &str, &str)] = &[
    ("sonnet", "anthropic/claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("opus", "anthropic/claude-opus-4-6", "Claude Opus 4.6"),
    ("haiku", "anthropic/claude-haiku-4-5", "Claude Haiku 4.5"),
    ("gpt4o", "openai/gpt-4o", "GPT-4o"),
    ("deepseek", "deepseek/deepseek-chat", "DeepSeek Chat"),
    ("gemini", "google/gemini-2.5-flash", "Gemini 2.5 Flash"),
];
```

This works on all providers because `parse_provider_model()` handles the prefix correctly:
- On Anthropic: splits on `/`, parses `anthropic` as `ProviderKind` → `(Anthropic, "claude-sonnet-4-6")`
- On OpenRouter: `model_names_contain_slash()` is true → treats whole string as model name → `(OpenRouter, "anthropic/claude-sonnet-4-6")` which is a valid OpenRouter model

Also fixed the "Already using" check in `switch_model()` to compare resolved `(provider, model)` tuples instead of display strings, and updated the `/model` args_hint from `[sonnet|opus|haiku]` to `[<name|alias>]`.

## Prevention

- When adding new aliases to `MODEL_ALIASES`, always include the provider prefix (e.g., `anthropic/`, `openai/`). All aliases should be uniform.
- The `parse_provider_model()` function is the arbiter of how aliases resolve on different providers — test new aliases on both Anthropic and OpenRouter (the two provider types: bare names vs slash names).
- Related: `docs/solutions/ui-bugs/tui-provider-model-config-state-divergence.md` (#442), `docs/solutions/ui-bugs/tui-provider-switch-worker-propagation.md` (#451).
