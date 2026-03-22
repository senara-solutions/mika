---
title: "feat: Per-provider LLM config"
type: feat
status: active
date: 2026-03-22
origin: docs/brainstorms/2026-03-22-per-provider-llm-config-brainstorm.md
---

# Per-Provider LLM Config

## Overview

Replace the current flat `llm_api_key` / `llm_model` / `llm_base_url` config with a provider-first design. A new `llm_provider` key selects the active provider; each provider gets its own group of three fields (`api_key`, `model`, `base_url`). `ModelSpec::parse()` prefix routing is deleted. No backward compatibility (pre-1.0 breaking change).

## Problem Statement / Motivation

The current config assumes a single LLM provider. Users set `llm_model = "openai/gpt-4o"` with a `provider/model` prefix to route, but this overloads one field with two concerns (provider selection + model choice). API keys and base URLs are shared across providers (`MIKA_LLM_API_KEY` serves all providers), which is wrong when users configure multiple providers (e.g., Anthropic for chat, OpenAI for embeddings). The flat design cannot express "I have keys for Anthropic and OpenAI, use Anthropic for LLM".

## Proposed Solution

Provider-first config with flat fields:

```toml
# ~/.mika/agents/mika-dev/config.toml
llm_provider = "anthropic"
llm_anthropic_model = "claude-sonnet-4-6"
```

```bash
# ~/.mika/.env
MIKA_LLM_ANTHROPIC_API_KEY=sk-ant-...
MIKA_LLM_OPENAI_API_KEY=sk-...
```

At runtime: read `llm_provider` → pick the matching group → pass `(api_key, model, base_url)` to `create_provider()`.

## Technical Approach

### Phase 1: Core Config Refactor (`crates/mika-common/`)

The foundation — `Settings` struct, config registry, LLM provider construction. Everything downstream depends on this.

#### 1a. Extend `ProviderKind` in `crates/mika-common/src/llm/mod.rs`

Extend the existing `ProviderKind` enum (do **not** create a new enum — the variants are identical, and a new `LlmProvider` enum would shadow the existing `LlmProvider` trait):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Ollama,
    Groq,
    MiniMax,
    Qwen,
    Kimi,
    OpenAiCompatible,
}
```

Add to existing enum:
- `Copy` derive (needed for `const MODEL_ALIASES` array)
- `Display` — lowercase: `"anthropic"`, `"openai"`, `"ollama"`, `"groq"`, `"minimax"`, `"qwen"`, `"kimi"`, `"openai-compatible"`
- `FromStr` — same strings, case-insensitive
- `serde::Deserialize` — delegates to `FromStr` via `#[serde(try_from = "String")]` or manual impl
- `ProviderKind::config_prefix(&self) -> &'static str` — returns field prefix: `"anthropic"`, `"openai"`, `"ollama"`, `"groq"`, `"minimax"`, `"qwen"`, `"kimi"`, `"openai_compatible"` (note: underscore for `openai_compatible` in field names)

The `Settings` struct uses `llm_provider: ProviderKind` directly. No mapping layer needed.

#### 1b. `Settings` struct changes in `crates/mika-common/src/config.rs`

**Remove fields:**
- `llm_model: String`
- `llm_api_key: Option<String>`
- `llm_base_url: Option<String>`
- `default_llm_model()` function

**Add fields:**
```rust
pub llm_provider: ProviderKind,  // required, no #[serde(default)]

// Anthropic
pub llm_anthropic_api_key: Option<String>,
pub llm_anthropic_model: Option<String>,
pub llm_anthropic_base_url: Option<String>,

// OpenAI
pub llm_openai_api_key: Option<String>,
pub llm_openai_model: Option<String>,
pub llm_openai_base_url: Option<String>,

// Ollama
pub llm_ollama_api_key: Option<String>,
pub llm_ollama_model: Option<String>,
pub llm_ollama_base_url: Option<String>,

// Groq
pub llm_groq_api_key: Option<String>,
pub llm_groq_model: Option<String>,
pub llm_groq_base_url: Option<String>,

// MiniMax
pub llm_minimax_api_key: Option<String>,
pub llm_minimax_model: Option<String>,
pub llm_minimax_base_url: Option<String>,

// Qwen
pub llm_qwen_api_key: Option<String>,
pub llm_qwen_model: Option<String>,
pub llm_qwen_base_url: Option<String>,

// Kimi
pub llm_kimi_api_key: Option<String>,
pub llm_kimi_model: Option<String>,
pub llm_kimi_base_url: Option<String>,

// OpenAI-Compatible
pub llm_openai_compatible_api_key: Option<String>,
pub llm_openai_compatible_model: Option<String>,
pub llm_openai_compatible_base_url: Option<String>,
```

**Env var mapping:** config-rs `Environment::with_prefix("MIKA")` auto-maps:
- `MIKA_LLM_PROVIDER` → `llm_provider`
- `MIKA_LLM_ANTHROPIC_API_KEY` → `llm_anthropic_api_key`
- `MIKA_LLM_OPENAI_COMPATIBLE_BASE_URL` → `llm_openai_compatible_base_url`

Single underscores are literal field separators; double underscores (`__`) denote nesting. All fields are flat — no nesting issues.

#### 1c. `active_llm_config()` method on `Settings`

```rust
/// Returns (api_key, model, base_url) for the active LLM provider.
/// Validates required fields per provider — errors name the specific missing config key.
pub fn active_llm_config(&self) -> Result<(Option<String>, String, Option<String>)> {
    // TODO: llm-auto-select-middleware will override `llm_provider` here
    match self.llm_provider {
        ProviderKind::Anthropic => {
            let api_key = self.llm_anthropic_api_key.clone()
                .ok_or_else(|| anyhow!("llm_anthropic_api_key is required (set MIKA_LLM_ANTHROPIC_API_KEY or add to ~/.mika/.env)"))?;
            let model = self.llm_anthropic_model.clone()
                .ok_or_else(|| anyhow!("llm_anthropic_model is required when llm_provider = \"anthropic\""))?;
            Ok((Some(api_key), model, self.llm_anthropic_base_url.clone()))
        }
        ProviderKind::OpenAi | ProviderKind::Groq | ProviderKind::MiniMax
        | ProviderKind::Qwen | ProviderKind::Kimi => {
            // All cloud providers: api_key required, model required, base_url optional (has default)
            let prefix = self.llm_provider.config_prefix();
            let (api_key, model, base_url) = self.provider_fields(prefix);
            let api_key = api_key.ok_or_else(|| anyhow!(
                "llm_{prefix}_api_key is required (set MIKA_LLM_{}_API_KEY or add to ~/.mika/.env)",
                prefix.to_uppercase()
            ))?;
            let model = model.ok_or_else(|| anyhow!(
                "llm_{prefix}_model is required when llm_provider = \"{}\"", self.llm_provider
            ))?;
            Ok((Some(api_key), model, base_url))
        }
        ProviderKind::Ollama => {
            let model = self.llm_ollama_model.clone()
                .ok_or_else(|| anyhow!("llm_ollama_model is required when llm_provider = \"ollama\""))?;
            // api_key optional for Ollama (local provider)
            Ok((self.llm_ollama_api_key.clone(), model, self.llm_ollama_base_url.clone()))
        }
        ProviderKind::OpenAiCompatible => {
            let model = self.llm_openai_compatible_model.clone()
                .ok_or_else(|| anyhow!("llm_openai_compatible_model is required when llm_provider = \"openai-compatible\""))?;
            let base_url = self.llm_openai_compatible_base_url.clone()
                .ok_or_else(|| anyhow!("llm_openai_compatible_base_url is required when llm_provider = \"openai-compatible\""))?;
            // api_key optional (some custom endpoints don't need auth)
            Ok((self.llm_openai_compatible_api_key.clone(), model, Some(base_url)))
        }
    }
}

/// Helper: extract the three fields for a provider by config prefix.
/// Used by the grouped cloud-provider match arm.
fn provider_fields(&self, prefix: &str) -> (Option<String>, Option<String>, Option<String>) {
    match prefix {
        "openai" => (self.llm_openai_api_key.clone(), self.llm_openai_model.clone(), self.llm_openai_base_url.clone()),
        "groq" => (self.llm_groq_api_key.clone(), self.llm_groq_model.clone(), self.llm_groq_base_url.clone()),
        "minimax" => (self.llm_minimax_api_key.clone(), self.llm_minimax_model.clone(), self.llm_minimax_base_url.clone()),
        "qwen" => (self.llm_qwen_api_key.clone(), self.llm_qwen_model.clone(), self.llm_qwen_base_url.clone()),
        "kimi" => (self.llm_kimi_api_key.clone(), self.llm_kimi_model.clone(), self.llm_kimi_base_url.clone()),
        _ => unreachable!("provider_fields called with unknown prefix: {prefix}"),
    }
}
```

**Validation rules per provider:**
- **Anthropic:** `api_key` required, `model` required, `base_url` optional
- **Cloud providers (OpenAi, Groq, MiniMax, Qwen, Kimi):** `api_key` required, `model` required, `base_url` optional (has built-in default)
- **Ollama:** `api_key` optional (local), `model` required, `base_url` optional (has built-in default)
- **OpenAiCompatible:** `api_key` optional, `model` required, `base_url` required (no built-in default)

All validation happens in `active_llm_config()` — not deferred to `create_provider()`. This ensures errors name the specific config key and env var to set.

#### 1d. `make_llm_provider()` rewrite

```rust
pub fn make_llm_provider(&self) -> anyhow::Result<Arc<dyn LlmProvider>> {
    let (api_key, model, base_url) = self.active_llm_config()?;
    let spec = ModelSpec {
        provider: self.llm_provider,
        model,
        base_url,
        api_key,
    };
    crate::llm::create_provider(&spec, self.llm_max_tokens)
}
```

**Delete `ModelSpec::parse()`** and all its tests. The `ModelSpec` struct itself is retained as an internal transport between config and provider construction — just remove the `parse()` factory. `with_base_url()` and `with_api_key()` builders are also deleted (fields set directly).

#### 1e. `CONFIG_KEYS` registry update

**Remove:**
- `llm_model` (File)
- `llm_base_url` (File)
- `llm_api_key` (Env, secret)

**Add:**
```rust
ConfigKeyInfo { key: "llm_provider", backend: ReadOnly, env_var: Some("MIKA_LLM_PROVIDER"), secret: false, description: "Active LLM provider" },

// Per provider (repeat for each of 8 providers):
ConfigKeyInfo { key: "llm_anthropic_api_key", backend: Env, env_var: Some("MIKA_LLM_ANTHROPIC_API_KEY"), secret: true, description: "Anthropic API key" },
ConfigKeyInfo { key: "llm_anthropic_model", backend: File, env_var: Some("MIKA_LLM_ANTHROPIC_MODEL"), secret: false, description: "Anthropic model name" },
ConfigKeyInfo { key: "llm_anthropic_base_url", backend: File, env_var: Some("MIKA_LLM_ANTHROPIC_BASE_URL"), secret: false, description: "Anthropic API base URL override" },
// ... (openai, ollama, groq, minimax, qwen, kimi, openai_compatible)
```

Total: 1 (`llm_provider`) + 8 × 3 = 25 new entries replacing 3 old ones.

#### 1f. `get_effective_value()` match arms

Add match arms for all 25 new keys. Pattern per provider:
```rust
"llm_provider" => Some(settings.llm_provider.to_string()),
"llm_anthropic_api_key" => settings.llm_anthropic_api_key.clone(),
"llm_anthropic_model" => settings.llm_anthropic_model.clone(),
"llm_anthropic_base_url" => settings.llm_anthropic_base_url.clone(),
// ... repeat for each provider
```

The existing coverage test (`test_get_effective_value_covers_all_non_db_non_env_keys`) catches missing branches.

#### 1g. `Debug` impl redaction

Redact all 8 `_api_key` fields:
```rust
.field("llm_anthropic_api_key", &self.llm_anthropic_api_key.as_ref().map(|_| "[REDACTED]"))
.field("llm_openai_api_key", &self.llm_openai_api_key.as_ref().map(|_| "[REDACTED]"))
// ... all 8 provider api_key fields
```

#### 1h. `validation.rs` updates

**Remove match arms:** `"llm_model"`, `"llm_base_url"`

**Add match arms for each `llm_{provider}_model`:** Same validation as old `llm_model` (non-empty, non-whitespace).

**Add match arms for each `llm_{provider}_base_url`:** Same validation as old `llm_base_url` (http/https scheme, non-empty).

**Add match arm for `"llm_provider"`:** Validate it's a known provider string via `ProviderKind::from_str()`.

#### 1i. `home.rs` default config template

Update `DEFAULT_CONFIG`:
```rust
const DEFAULT_CONFIG: &str = r#"llm_provider = "anthropic"
llm_anthropic_model = "claude-sonnet-4-6"
"#;
```

`DEFAULT_GLOBAL_CONFIG` (`log_level = "info"`) — unchanged.

#### 1j. Config cascade tests in `config.rs`

Rewrite all tests that parse config.toml strings with `llm_model`. Example:
```rust
// Before:
"llm_model = \"claude-opus-4-6\"\nlog_level = \"debug\"\n"
// After:
"llm_provider = \"anthropic\"\nllm_anthropic_model = \"claude-opus-4-6\"\nlog_level = \"debug\"\n"
```

Update agent cascade tests (global vs agent override) to test `llm_provider` override.

### Phase 2: CLI & TUI Updates (`crates/mika-cli/`)

#### 2a. `setup.rs` — Provider selection prompt

**`run_cli_prompts()`:** Add provider selection as first prompt:
```rust
let providers = &["anthropic", "openai", "ollama", "groq", "minimax", "qwen", "kimi", "openai-compatible"];
let selection = Select::new()
    .with_prompt("LLM provider")
    .items(providers)
    .default(0)  // anthropic
    .interact()?;
let provider = providers[selection];
```

Then prompt for the provider-specific API key (skip for Ollama). Write `llm_provider` to `config.toml`, API key to `.env` with the provider-specific var name.

**`run_oauth_setup()`:** Check `MIKA_LLM_ANTHROPIC_API_KEY` instead of `MIKA_LLM_API_KEY`. If `llm_provider` is set and not `"anthropic"`, warn: "OAuth setup only applies to Anthropic provider."

**`run_compose_generation()`:** Add provider prompt. Write `MIKA_LLM_PROVIDER` and `MIKA_LLM_{PROVIDER}_API_KEY` to compose `.env`.

**`--api-key` flag:** When provided without `--provider`, default to `"anthropic"` (most common case). Add `--provider` flag to `setup` subcommand for explicit control.

#### 2b. `doctor.rs` — Provider-aware checks

**`check_api_key()`:** Read `llm_provider` from settings → derive the env var name (`MIKA_LLM_{PROVIDER}_API_KEY`) → check env + `.env` file. For Ollama, report "API key: not required (local provider)" and return OK. For OpenAI-compatible, additionally check that `base_url` is configured.

**`check_optional_key()`:** Unchanged (covers `MIKA_OPENAI_API_KEY`, `MIKA_BRAVE_API_KEY`, etc.).

#### 2c. `config.rs` — Config display updates

**`show_config()`:** Replace:
```rust
// Before:
println!("  Model:      {}", ctx.settings.llm_model);
// After:
println!("  Provider:   {}", ctx.settings.llm_provider);
match ctx.settings.active_llm_config() {
    Ok((_, model, _)) => println!("  Model:      {}", model),
    Err(e) => println!("  Model:      <error: {}>", e),
}
```

**`mika config list` display:** Show `llm_provider` plus only the active provider's three keys. Add a note: "Use `mika config list --all` to see all provider keys." (Optional enhancement — can defer to just showing all keys if simpler.)

#### 2d. `cli.rs` — `MODEL_ALIASES` refactor

Replace flat alias array with per-provider structure:

```rust
pub const MODEL_ALIASES: &[(ProviderKind, &str, &str, &str)] = &[
    // (provider, alias, model_id, display_name)
    (ProviderKind::Anthropic, "sonnet", "claude-sonnet-4-6", "Claude Sonnet 4.6"),
    (ProviderKind::Anthropic, "opus", "claude-opus-4-6", "Claude Opus 4.6"),
    (ProviderKind::Anthropic, "haiku", "claude-haiku-4-5", "Claude Haiku 4.5"),
    (ProviderKind::MiniMax, "minimax", "MiniMax-M2.5", "MiniMax M2.5"),
    (ProviderKind::Qwen, "qwen", "qwen3.5-plus", "Qwen 3.5 Medium"),
    (ProviderKind::Kimi, "kimi", "kimi-k2.5", "Kimi K2.5"),
];
```

**`resolve_model_alias()`:** Filter by active provider. If alias matches a different provider, return the input unchanged (no cross-provider aliasing).

#### 2e. `init.rs` — `override_model()` rewrite

```rust
pub fn override_model(&mut self, model: &str) -> Result<()> {
    let resolved = crate::cli::resolve_model_alias(model, &self.db_ctx.settings.llm_provider);
    // Set the active provider's model field
    match self.db_ctx.settings.llm_provider {
        ProviderKind::Anthropic => self.db_ctx.settings.llm_anthropic_model = Some(resolved),
        ProviderKind::OpenAi => self.db_ctx.settings.llm_openai_model = Some(resolved),
        // ... etc
    }
    self.llm = self.db_ctx.settings.make_llm_provider()?;
    Ok(())
}
```

**`--model` with provider prefix:** Reject values containing `/` with an error: "Provider prefixes are no longer supported in --model. Use `llm_provider` config or /provider TUI command to switch providers."

#### 2f. `tui/commands/handlers.rs` — `/model` rewrite

**`handle_model()`:** Show models for active provider only. List aliases for that provider. Accept free-text model names. Persist to `llm_{provider}_model` in config.toml via `write_config_toml()`.

#### 2g. `tui/commands/handlers.rs` — New `/provider` command

```rust
fn handle_provider(args: &str, ctx: &mut TuiContext) -> CommandResult {
    if args.is_empty() {
        // List all providers with current marked
        let providers = ["anthropic", "openai", "ollama", "groq", "minimax", "qwen", "kimi", "openai-compatible"];
        for p in &providers {
            let marker = if p == &ctx.settings.llm_provider.to_string() { " (active)" } else { "" };
            println!("  {}{}", p, marker);
        }
        return Ok(());
    }

    let new_provider = ProviderKind::from_str(args)?;

    // Check if the new provider's API key is configured (skip for Ollama)
    let test_settings = /* clone settings, set llm_provider = new_provider */;
    match test_settings.active_llm_config() {
        Ok(_) => {
            // Persist to config.toml
            write_config_toml(&config_path, "llm_provider", args)?;
            ctx.settings.llm_provider = new_provider;
            // Rebuild LLM provider
            ctx.llm = ctx.settings.make_llm_provider()?;
        }
        Err(e) => {
            return Err(anyhow!("Cannot switch to {}: {}. Configure the required keys first.", args, e));
        }
    }
    Ok(())
}
```

Register in `COMMANDS` array. Add to `TEAM_MODE_BLOCKED_COMMANDS` (provider switching during team run is dangerous). Add tab completion.

#### 2h. `chat.rs` — `AgentRequest::SetModel` update

The chat worker receives `AgentRequest::SetModel { model }`. Update to set the provider-specific field instead of `llm_model`:
```rust
AgentRequest::SetModel { model } => {
    let mut updated_settings = worker_settings.clone();
    match updated_settings.llm_provider {
        ProviderKind::Anthropic => updated_settings.llm_anthropic_model = Some(model),
        // ... etc
    }
    if let Ok(new_llm) = updated_settings.make_llm_provider() {
        worker_llm = new_llm;
    }
}
```

### Phase 3: Handler Scripts & Security

#### 3a. `run.sh` unset list update

Replace `MIKA_LLM_API_KEY` with all provider API keys:
```bash
unset MIKA_LLM_ANTHROPIC_API_KEY MIKA_LLM_OPENAI_API_KEY MIKA_LLM_OLLAMA_API_KEY \
      MIKA_LLM_GROQ_API_KEY MIKA_LLM_MINIMAX_API_KEY MIKA_LLM_QWEN_API_KEY \
      MIKA_LLM_KIMI_API_KEY MIKA_LLM_OPENAI_COMPATIBLE_API_KEY \
      MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY MIKA_INVESTIGATE_GITHUB_TOKEN
```

Update all handler scripts that have unset lists:
- `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`
- Check other handlers in `templates/skills/*/handlers/`

#### 3b. Rust-level scrubbing verification

`scrub_mika_env_vars()` in `crates/mika-agent/src/skills/executor.rs` already scrubs all `MIKA_*` vars by prefix. **No changes needed** — new `MIKA_LLM_{PROVIDER}_*` vars are covered automatically.

#### 3c. OAuth detection scoping

Move `is_oauth_token()` check to where `llm_anthropic_api_key` is consumed (in `active_llm_config()` or in the Anthropic provider constructor). Ensure it only fires for the Anthropic provider path.

### Phase 4: Documentation & Tests

#### 4a. `.env.example` update

Replace:
```bash
# Before:
MIKA_LLM_API_KEY=sk-ant-...

# After:
# LLM Provider (required): anthropic, openai, ollama, groq, minimax, qwen, kimi, openai-compatible
MIKA_LLM_PROVIDER=anthropic

# Anthropic
MIKA_LLM_ANTHROPIC_API_KEY=sk-ant-...
# MIKA_LLM_ANTHROPIC_MODEL=claude-sonnet-4-6
# MIKA_LLM_ANTHROPIC_BASE_URL=

# OpenAI
# MIKA_LLM_OPENAI_API_KEY=sk-...
# MIKA_LLM_OPENAI_MODEL=gpt-4o

# Ollama (local, no API key needed)
# MIKA_LLM_OLLAMA_MODEL=llama3
# MIKA_LLM_OLLAMA_BASE_URL=http://localhost:11434/v1

# ... remaining providers
```

#### 4b. `CLAUDE.md` Environment Variables section

Update the `MIKA_LLM_API_KEY` section to document `MIKA_LLM_PROVIDER` and per-provider keys. Update the Stack section to reflect provider-first routing instead of prefix-based.

#### 4c. `docs/configuration.md`

Update the settings reference table, config cascade examples, and any `llm_model` / `llm_api_key` references.

#### 4d. Test fixture updates

**`crates/mika-agent/src/test_utils.rs` — `dummy_settings()`:**
```rust
Settings {
    llm_provider: ProviderKind::Anthropic,
    llm_anthropic_api_key: None,
    llm_anthropic_model: Some("claude-sonnet-4-6".to_string()),
    llm_anthropic_base_url: None,
    llm_openai_api_key: None,
    llm_openai_model: None,
    llm_openai_base_url: None,
    // ... all 24 Option<String> fields set to None (except anthropic model)
    llm_max_tokens: 4096,
    // ... rest unchanged
}
```

**`crates/mika-agent/src/server/mod.rs` — `test_state()`:** Same pattern.

#### 4e. Delete `ModelSpec::parse()` tests

Remove all tests in `crates/mika-common/src/llm/mod.rs` that test `ModelSpec::parse()` (approximately 15 tests). Add new tests for `ProviderKind::from_str()` and `active_llm_config()`.

### Phase 5: Cleanup & Verification

#### 5a. Delete dead code

- `ModelSpec::parse()` function
- `ModelSpec::with_base_url()` and `with_api_key()` builder methods
- `default_llm_model()` function in `config.rs`
- Provider prefix constants/matching in `ModelSpec::parse()`

#### 5b. Full verification

```bash
cargo build                    # Struct field changes caught by compiler
cargo test                     # Test fixtures + coverage test for get_effective_value
cargo clippy                   # No warnings
rg 'llm_api_key|llm_model|llm_base_url|MIKA_LLM_API_KEY|MIKA_LLM_MODEL|MIKA_LLM_BASE_URL' \
  --type rust --type sh --type md --type toml \
  | grep -v 'docs/brainstorms/' | grep -v 'docs/solutions/' | grep -v 'docs/plans/'
# Should return zero results (excluding historical docs)
```

## System-Wide Impact

### Interaction Graph

`Settings::load_for_agent()` → `make_llm_provider()` → `active_llm_config()` → `create_provider()` → `Arc<dyn LlmProvider>` stored in:
- `AppContext.llm` (CLI)
- `AppState.llm` (server)
- `AgentWorker` local var (chat)
- `TeamEngine` per-agent construction
- `SilentAgentParams.llm` (heartbeat, reflection, callback)
- `DelegatedAgentParams.llm` (delegate_task)

All consumers hold `Arc<dyn LlmProvider>` — no code change needed downstream of `make_llm_provider()`.

### Error Propagation

- **Missing `llm_provider`:** config-rs deserialization fails → `Settings::load_for_agent()` returns error → startup aborts with message. The error from config-rs for a missing required field is generic ("missing field `llm_provider`"). Consider wrapping with a better message: "Configuration key 'llm_provider' is required. Add `llm_provider = \"anthropic\"` to config.toml or set MIKA_LLM_PROVIDER env var. Run `mika setup` for guided configuration."
- **Missing model for active provider:** `active_llm_config()` returns `Err` → `make_llm_provider()` propagates → startup aborts with provider-specific message.
- **Missing API key for non-Ollama provider:** Same path, clear error naming the missing env var.

### State Lifecycle Risks

No persistent state changes. Config is read-only at runtime. The only writes are `mika setup` (to `.env` and `config.toml`) and `/provider`/`/model` TUI commands (to `config.toml`). No database migrations, no schema changes, no data conversion.

**Risk: stale config after upgrade.** Old `llm_model` and `llm_api_key` entries remain in config.toml and `.env`. config-rs ignores unknown keys silently — no crash, but confusing. **Mitigation:** `mika doctor` can warn about unrecognized keys (optional, can be a follow-up).

### API Surface Parity

| Interface | Change needed |
|-----------|--------------|
| `Settings::make_llm_provider()` | Rewritten (Phase 1d) |
| `mika config get/set/list` | Updated keys (Phase 2c) |
| `mika setup` | Provider prompt (Phase 2a) |
| `mika doctor` | Provider-aware checks (Phase 2b) |
| `--model` CLI flag | Within-provider only (Phase 2e) |
| `/model` TUI command | Per-provider (Phase 2f) |
| `/provider` TUI command | **New** (Phase 2g) |
| Server `/message` endpoint | No change (uses `AppState.llm`) |
| A2A endpoints | No change |

## Acceptance Criteria

### Functional Requirements

- [ ] `llm_provider` selects the active provider (8 variants)
- [ ] Per-provider `api_key`, `model`, `base_url` fields work via config.toml and env vars
- [ ] `active_llm_config()` returns correct tuple for each provider
- [ ] `api_key` optional for Ollama
- [ ] `base_url` required for OpenAI-compatible
- [ ] `make_llm_provider()` constructs the correct provider
- [ ] Old fields (`llm_model`, `llm_api_key`, `llm_base_url`) fully removed
- [ ] `ModelSpec::parse()` deleted
- [ ] `/provider` TUI command switches provider with validation
- [ ] `/model` shows models for active provider only
- [ ] `--model` overrides within active provider, rejects provider prefixes
- [ ] `mika setup` prompts for provider first
- [ ] `mika doctor` checks provider-specific keys
- [ ] OAuth detection scoped to Anthropic only
- [ ] `llm_max_tokens` unchanged
- [ ] Embedding config (`openai_api_key`, `embedding_model`, `embedding_base_url`) unchanged

### Non-Functional Requirements

- [ ] `cargo build` passes
- [ ] `cargo test` passes (all ~1518 tests)
- [ ] `cargo clippy` passes with no warnings
- [ ] No grep hits for old field/env var names (excluding historical docs)
- [ ] All `_api_key` fields redacted in `Debug` output
- [ ] Handler script `unset` lists updated
- [ ] `MIKA_*` prefix scrubbing covers new vars automatically (verify)

### Documentation Requirements

- [ ] `.env.example` updated
- [ ] `CLAUDE.md` Environment Variables section updated
- [ ] `docs/configuration.md` updated
- [ ] PR description documents migration steps for existing users

## Dependencies & Risks

**Risk: Large diff.** 24 new fields on `Settings` plus registry entries, match arms, and test fixtures. Mitigated by compiler enforcement — any missed field is a compile error in `dummy_settings()`.

**Risk: config-rs deserialization of `ProviderKind` enum.** Custom `Deserialize` must handle lowercase strings. Test with config.toml and env var sources.

**Risk: Test coverage gap.** Config cascade tests need rewriting, not just updating. Ensure global-override-agent-override-env precedence is tested for `llm_provider` and per-provider fields.

## Future Considerations

Single YAGNI foothold in `active_llm_config()`:
```rust
// TODO: llm-auto-select-middleware will override `llm_provider` here
```

No trait, no enum variant, no config key for auto-select. Just the comment.

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-22-per-provider-llm-config-brainstorm.md](docs/brainstorms/2026-03-22-per-provider-llm-config-brainstorm.md) — Key decisions: `/provider` TUI command, `openai_compatible` field prefix, Ollama api_key optional, no backward compat, enumerate all provider API keys in shell unset.

### Internal References

- Config key rename checklist: [docs/solutions/architecture-patterns/config-key-rename-across-layers.md](docs/solutions/architecture-patterns/config-key-rename-across-layers.md)
- Unified API key consolidation: [docs/solutions/architecture-patterns/unified-llm-api-key-consolidation.md](docs/solutions/architecture-patterns/unified-llm-api-key-consolidation.md)
- Config 4-source model: [docs/solutions/architecture-patterns/simplified-config-4-source-model.md](docs/solutions/architecture-patterns/simplified-config-4-source-model.md)
- Env var leakage: [docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md](docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md)
- Multi-provider trait: [docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md](docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md)
- CLI --model override: [docs/solutions/architecture-patterns/cli-model-override-one-shot.md](docs/solutions/architecture-patterns/cli-model-override-one-shot.md)
- Original multi-provider brainstorm: [docs/brainstorms/2026-03-13-multi-provider-llm-brainstorm.md](docs/brainstorms/2026-03-13-multi-provider-llm-brainstorm.md)

### Key Files

| File | Changes |
|------|---------|
| `crates/mika-common/src/config.rs` | Settings struct, CONFIG_KEYS, get_effective_value(), Debug impl, make_llm_provider(), tests |
| `crates/mika-common/src/llm/mod.rs` | Extend ProviderKind (Display/FromStr/Deserialize/Copy), delete ModelSpec::parse(), update create_provider() |
| `crates/mika-common/src/validation.rs` | New match arms for per-provider keys |
| `crates/mika-common/src/home.rs` | DEFAULT_CONFIG template |
| `crates/mika-cli/src/cli.rs` | MODEL_ALIASES, resolve_model_alias() |
| `crates/mika-cli/src/init.rs` | override_model() |
| `crates/mika-cli/src/commands/setup.rs` | Provider prompt, OAuth scoping, compose mode |
| `crates/mika-cli/src/commands/doctor.rs` | Provider-aware key checks |
| `crates/mika-cli/src/commands/config.rs` | show_config(), config list display |
| `crates/mika-cli/src/commands/chat.rs` | SetModel handler |
| `crates/mika-cli/src/tui/commands/handlers.rs` | /model rewrite, new /provider command |
| `crates/mika-agent/src/test_utils.rs` | dummy_settings() |
| `crates/mika-agent/src/server/mod.rs` | test_state() |
| `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh` | unset list |
| `.env.example` | Full rewrite of LLM section |
| `CLAUDE.md` | Environment Variables, Stack sections |
| `docs/configuration.md` | Settings reference table |
