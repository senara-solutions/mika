---
title: "fix: /provider and /model commands leave broken config state"
type: fix
status: completed
date: 2026-04-05
---

# fix: /provider and /model commands leave broken config state

## Overview

The TUI `/provider` and `/model` slash commands leave agent config in a broken state after switching providers. The core bugs are: (1) `/provider` always uses `default_model()` instead of reading the user's configured `{provider}_model` from Settings, (2) the model is not persisted to `config.toml` on provider switch, (3) no `llm_max_tokens` validation against provider limits, and (4) no feedback about stale config fields. Additionally, `/model` with no args only shows 6 hardcoded aliases instead of the provider's available models.

## Problem Statement

**Root cause (handlers.rs:631):** `handle_provider` calls `new_provider.default_model()` instead of loading the resolved model from `Settings::active_llm_config()`. This means:

1. If the user previously configured `deepseek_model = "deepseek-reasoner"` in config.toml, switching to DeepSeek ignores it and shows/uses `deepseek-chat`
2. Only `llm_provider` is persisted -- the model sent to the worker via `SetModel` is ephemeral
3. On restart, `active_llm_config()` correctly reads the configured model, creating a divergence between the TUI session and the persisted state
4. `llm_max_tokens = 16384` silently exceeds DeepSeek's 8192 limit
5. Stale `openrouter_model` fields from previous providers remain without any indication

**Prior art:** Issues #342-#344 established the validate-first pattern -- `/provider` and `/model` must call `validate_provider_switch_for()` BEFORE updating UI state. This fix builds on that foundation.

## Proposed Solution

Two phases: (1) fix the core provider/model state bugs, (2) add model list fetching and interactive picker.

### Phase 1: Fix provider switch state management

**1a. Read configured model from Settings instead of hardcoded default**

In `handle_provider` (handlers.rs:617-665), after `validate_provider_switch_for()` succeeds:

```rust
// Load settings with the new provider to get the resolved model
let mut settings = Settings::load_for_agent(&app.global_home, &app.home_dir)
    .map_err(|e| format!("failed to load settings: {e}"))?;
settings.llm_provider = new_provider;
let config = settings.active_llm_config();
let model = config.model.clone();
```

This uses the same `active_llm_config()` path that the agent loop uses, which falls back to `default_model()` only when no `{provider}_model` is configured.

**1b. Persist the model to config.toml when it's a default**

If no `{provider}_model` was configured (i.e., `active_llm_config()` fell back to `default_model()`), persist the default so the user can see what model will be used:

```rust
// If provider has no configured model, persist the default
let provider_model_key = format!("{}_model", new_provider.config_prefix());
if settings.provider_fields(new_provider).0.is_none() {
    let _ = write_config_toml(&config_path, &provider_model_key, model);
}
```

**1c. Warn about stale model fields**

After switching, check for configured model fields from the provider being switched away FROM (not all providers -- that would be noisy):

```rust
// Check if the old provider has a stale model field
let old_prefix = old_provider.config_prefix();
let old_model_key = format!("{old_prefix}_model");
if settings.provider_fields(old_provider).0.is_some() {
    stale_warning = format!("\nNote: {old_model_key} is still set (kept for switching back)");
}
```

**1d. Validate llm_max_tokens against provider-level limits**

Add a `max_output_tokens()` method on `ProviderKind` with conservative per-provider defaults:

```rust
impl ProviderKind {
    pub fn max_output_tokens(&self) -> u32 {
        match self {
            Self::Anthropic => 128_000,  // Claude extended output
            Self::OpenAi => 16_384,
            Self::OpenRouter => 128_000, // varies by model
            Self::Groq => 8_192,
            Self::Ollama => 131_072,     // no limit
            Self::Mistral => 8_192,
            Self::Google => 65_536,
            Self::DeepSeek => 8_192,
            Self::MiniMax => 16_384,
            Self::Kimi => 8_192,
            Self::Qwen => 8_192,
        }
    }
}
```

On provider switch, if `settings.llm_max_tokens > new_provider.max_output_tokens()`:
```
Warning: llm_max_tokens (16384) exceeds {provider}'s limit ({limit}). Consider: /provider set max_tokens {limit}
```

Warn only -- don't auto-clamp (user may know what they're doing with OpenRouter models).

**1e. Show what model will be used**

The output message already shows the model. Just ensure it shows the *resolved* model (from Settings), not the hardcoded default:

```
Switched to deepseek (model: deepseek-reasoner).
Note: openrouter_model is still set (kept for switching back)
```

### Phase 2: Model list fetching and interactive picker

**2a. Add `list_models()` to `ProviderKind`**

New function in `mika-common/src/llm/models.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: Option<String>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCache {
    pub provider: String,
    pub base_url: Option<String>,
    pub fetched_at: String, // ISO 8601
    pub models: Vec<ModelInfo>,
}
```

Per-provider fetch implementation:
- **OpenAI-compatible** (OpenAI, OpenRouter, Groq, Mistral, DeepSeek, MiniMax, Kimi, Qwen): `GET {base_url}/models` -- already used by `check_health()`. Parse `{ data: [{ id, ... }] }`.
- **Anthropic**: Hardcoded list (Claude Opus, Sonnet, Haiku variants). No `/models` API.
- **Ollama**: `GET {base_url}/api/tags` -- different shape `{ models: [{ name, ... }] }`.
- **Google**: Hardcoded list (Gemini models). Google AI API uses a different format.

`fetch_models(provider, base_url, api_key) -> Result<Vec<ModelInfo>>` with 10s timeout, returning `Err` on failure (caller falls back to free-text).

**2b. Cache per agent directory**

Cache file: `{agent_home}/cache/models/{provider_prefix}.json`
- TTL: 24 hours
- Invalidated when: provider's `base_url` changes (custom endpoint serves different models), manual `mika skills update` (resets all caches)
- Cache miss or stale: fetch fresh, write cache, return models
- Fetch failure: return stale cache if exists (any age), else return empty

```rust
pub async fn get_or_fetch_models(
    agent_home: &Path,
    provider: ProviderKind,
    base_url: Option<&str>,
    api_key: Option<&str>,
) -> Vec<ModelInfo>
```

**2c. `/model` with no args: show available models**

Replace the current "Unknown model" error with a model list display:

```
Available models for deepseek:
  deepseek-chat (current)
  deepseek-reasoner

Aliases: sonnet, opus, haiku, gpt4o, deepseek, gemini
Usage: /model <name> or /model <alias>
```

If the model list is from cache, show `[cached]`. If fetch failed and no cache, show aliases only (current behavior).

**Not building an interactive picker.** The issue spec suggested `dialoguer select`, but:
1. `dialoguer` is a terminal library incompatible with ratatui's terminal backend
2. A native ratatui popup widget is significant UI work unrelated to the core bug
3. Showing the list and letting the user type `/model <name>` is simpler and sufficient

The user can see all models and pick by name. This matches how `/provider` already works (list + type name).

**2d. `/model <alias>` cross-provider fix**

Currently `/model deepseek` sets `deepseek_model` but does NOT update `app.provider` or persist `llm_provider`. Fix: when an alias resolves to a different provider, run the same provider-switch logic as `/provider`:

```rust
if resolved_provider != app.provider {
    // Same validation + state update as handle_provider
    validate_provider_switch_for(&app.home_dir, &app.global_home, resolved_provider)?;
    write_config_toml(&config_path, "llm_provider", &resolved_provider.to_string())?;
    app.provider = resolved_provider;
}
```

## Acceptance Criteria

- [x] `/provider deepseek` uses configured `deepseek_model` from config.toml if set, falls back to `default_model()` if not
- [x] `/provider deepseek` persists the default model to config.toml when no `deepseek_model` was previously configured
- [x] Switching away from a provider that has a configured model shows a one-line note about the stale field
- [x] Switching to a provider where `llm_max_tokens` exceeds the provider's limit shows a warning
- [x] `/model` with no args shows available models for the current provider (fetched from API or hardcoded)
- [x] Model list is cached per-agent with 24h TTL, graceful fallback to stale cache or aliases-only on fetch failure
- [x] `/model <cross-provider-alias>` (e.g., `/model deepseek` while on Anthropic) also switches the active provider and persists `llm_provider`
- [x] All changes follow the validate-first pattern (validate before UI mutation)
- [x] Existing TUI handler tests pass; new tests added for each fixed scenario

## Technical Considerations

**Files to modify:**
- `crates/mika-cli/src/tui/commands/handlers.rs` — `handle_provider`, `handle_model` fixes
- `crates/mika-common/src/llm/mod.rs` — `ProviderKind::max_output_tokens()`
- `crates/mika-common/src/llm/models.rs` (new) — `ModelInfo`, `ModelCache`, `fetch_models()`, `get_or_fetch_models()`

**Three-file update rule** (from learnings): slash command changes must touch `commands/mod.rs`, `commands/handlers.rs`, and `commands/completers.rs`. The completer for `/model` should be updated to include fetched model names for tab completion.

**Test coverage:** The `TestApp` builder has 19 existing tests for provider/model handlers. Add tests for:
- Provider switch reads configured model from Settings
- Provider switch with no configured model uses and persists default
- Stale field warning appears when switching away from configured provider
- max_tokens warning when exceeding provider limit
- `/model` cross-provider alias switches provider
- Model list fetch (mock HTTP) and cache hit/miss/stale

**Scope boundaries:**
- CLI `mika config set llm_provider` is out of scope (follow-up issue)
- No ratatui interactive picker widget -- show list, user types name
- Per-model max_tokens accuracy is best-effort (hardcoded provider-level defaults, not per-model from API)
- Model list for Anthropic and Google is hardcoded (no public list API)

## Sources

- Prior fix: `docs/solutions/ui-bugs/tui-slash-command-reliability-clear-provider-model.md` (#342-#344)
- Architecture: `docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md`
- Config pattern: `docs/solutions/architecture-patterns/cli-model-override-one-shot.md`
- Related issue: #442
