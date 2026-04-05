---
title: "TUI /provider and /model commands leave broken config state"
category: ui-bugs
date: 2026-04-05
severity: high
tags: [tui, provider, model, config, state-divergence, slash-commands]
issues: ["#442"]
related:
  - "docs/solutions/ui-bugs/tui-slash-command-reliability-clear-provider-model.md"
  - "docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md"
  - "docs/solutions/architecture-patterns/cli-model-override-one-shot.md"
---

# TUI /provider and /model commands leave broken config state

## Problem

The TUI `/provider` command used `ProviderKind::default_model()` instead of reading the user's configured `{provider}_model` from Settings. This caused:

1. **Model ignored on switch:** `/provider deepseek` always showed/used `deepseek-chat` even if `deepseek_model = "deepseek-reasoner"` was in config.toml
2. **Model not persisted:** Only `llm_provider` was written to config.toml — the model sent to the worker was ephemeral, creating session/restart divergence
3. **No max_tokens validation:** `llm_max_tokens = 16384` silently exceeded DeepSeek's 8192 limit
4. **No stale field feedback:** Old `openrouter_model` fields remained without any indication

Additionally, `/model <alias>` with a cross-provider alias (e.g., `/model deepseek` while on Anthropic) set the model but did NOT switch the active provider or persist `llm_provider`, breaking on restart.

## Root Cause

`handlers.rs:631` — `handle_provider` called `new_provider.default_model()` directly instead of going through `Settings::active_llm_config()`, which is the canonical resolution path that falls back to `default_model()` only when no `{provider}_model` is configured.

This was a shortcut that bypassed the config cascade, creating a divergence between what the TUI displayed and what the agent engine would use after restart.

## Solution

### 1. Read configured model from Settings

Changed `validate_provider_switch_for()` to return the loaded `Settings` (instead of `()`) so callers can read `active_llm_config()` without a second load:

```rust
fn validate_provider_switch_for(
    home_dir: &Path,
    global_home: &Path,
    provider: ProviderKind,
) -> Result<Settings, String> {
    let mut settings = Settings::load_for_agent(global_home, home_dir)
        .map_err(|e| format!("failed to load settings: {e}"))?;
    settings.llm_provider = provider;
    settings.make_llm_provider().map_err(|e| e.to_string())?;
    Ok(settings)
}
```

Then in `handle_provider`, read the resolved model:

```rust
let config = settings.active_llm_config();
let model = config.model.clone();
```

### 2. Persist default model when none configured

```rust
if !had_configured_model {
    let model_key = format!("{}_model", new_provider.config_prefix());
    let _ = write_config_toml(&config_path, &model_key, &model);
}
```

### 3. Stale field and max_tokens warnings

```rust
// Stale field from provider being switched FROM
if settings.provider_fields(old_provider).0.is_some() {
    notes.push_str(&format!(
        "\nNote: {old_key} is still set (kept for switching back)"
    ));
}

// max_tokens exceeds new provider's limit
let max = new_provider.max_output_tokens();
if settings.llm_max_tokens > max {
    notes.push_str(&format!(
        "\nWarning: llm_max_tokens ({}) exceeds {}'s limit ({})",
        settings.llm_max_tokens, new_provider, max
    ));
}
```

### 4. Cross-provider /model alias fix

When `/model <alias>` resolves to a different provider, now also switches provider:

```rust
if target_provider != app.provider {
    write_config_toml(&config_path, "llm_provider", &target_provider.to_string())?;
    app.provider = target_provider;
}
```

### 5. Model list fetching and caching

New `mika-common/src/llm/models.rs` module fetches available models from provider APIs:
- **OpenAI-compatible providers** (OpenAI, DeepSeek, Groq, etc.): `GET {base_url}/models`
- **Ollama**: `GET {base_url}/api/tags`
- **Anthropic, Google**: Hardcoded lists (no public model list API)

Cache stored per-agent at `{agent_home}/cache/models/{provider}.json` with 24h TTL. Graceful fallback: fresh cache → API fetch → stale cache → empty.

### 6. apply_model_switch helper

Extracted common logic between alias and direct-name paths into a single `apply_model_switch()` helper to eliminate ~60 LOC duplication.

## Prevention

1. **Use `active_llm_config()` for model resolution** — never call `default_model()` directly when the user's config should be consulted
2. **Follow the override-then-reconstruct pattern** — when a config field controls object construction, always mutate + rebuild in a single path (documented in `cli-model-override-one-shot.md`)
3. **Validate-first pattern** — call `validate_provider_switch_for()` before any state mutation, as established in #342-344
4. **Persist what you display** — if the TUI shows a value to the user, that value should survive restart

## Key Files

- `crates/mika-cli/src/tui/commands/handlers.rs` — `handle_provider`, `handle_model`, `apply_model_switch`, `validate_provider_switch_for`
- `crates/mika-common/src/llm/mod.rs` — `ProviderKind::max_output_tokens()`
- `crates/mika-common/src/llm/models.rs` — `ModelInfo`, `ModelCache`, `get_models()`, `fetch_models_from_api()`
- `crates/mika-cli/src/tui/commands/completers.rs` — `complete_model()` with cached models
