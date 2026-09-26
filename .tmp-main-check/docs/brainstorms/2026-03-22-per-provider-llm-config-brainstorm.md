# Per-Provider LLM Config

**Date:** 2026-03-22
**Status:** Brainstorm (spec review)

## What We're Building

Replace the current single-key `llm_api_key` / `llm_model` / `llm_base_url` flat config with a provider-first design. Each provider gets its own group of three fields (`api_key`, `model`, `base_url`). A new `llm_provider` key selects the active provider at runtime. No backward compatibility.

## Spec Review: Edge Cases & Decisions

### 1. `--model` flag and `/model` TUI command

**Problem:** The spec deletes `ModelSpec::parse()` and removes `llm_model`. The `--model` CLI flag, `/model` TUI command, and `MODEL_ALIASES` all rely on this.

**Decision:** New `/provider` TUI command sets the default provider. `/model` displays models for the active provider. `--model` overrides only the model within the active provider. `MODEL_ALIASES` become per-provider (no more `provider/model` alias strings).

### 2. `OpenAiCompatible` variant naming

**Problem:** `ProviderKind::OpenAiCompatible` needs field names and env vars. Hyphens can't appear in Rust field names.

**Decision:** `openai-compatible` is a provider like any other. Canonical names:
- Display/FromStr: `"openai-compatible"`
- Rust field prefix: `llm_openai_compatible_*`
- Env var prefix: `MIKA_LLM_OPENAI_COMPATIBLE_*`
- `base_url` is required (not optional) for this variant — enforced in `active_llm_config()`

### 3. Ollama doesn't need an API key

**Problem:** `active_llm_config()` would error if `api_key` is missing, but Ollama runs locally without auth.

**Decision:** `api_key` is optional for Ollama. `active_llm_config()` returns `(Option<String>, String, Option<String>)` or does provider-specific validation.

### 4. `setup.rs` and `doctor.rs` flows

**Problem:** These directly read `MIKA_LLM_API_KEY` from env and `.env` file.

**Decision:** No backward compatibility. Update to read `llm_provider` first, then check the provider-specific key. `run_oauth_setup()` checks `MIKA_LLM_ANTHROPIC_API_KEY`. `is_oauth_token()` applies only when `llm_provider == anthropic`.

### 5. Handler script env var scrubbing

**Problem:** `run.sh` currently unsets `MIKA_LLM_API_KEY`. With N providers, need N keys.

**Decision:** Rust-level exec handler executor already does `env_clear()` + allowlist (scrubs all `MIKA_*`). The `run.sh` defense-in-depth unset detects provider context where possible; static list of all provider API keys as fallback safety net.

### Additional areas requiring changes (no design decisions needed)

- `validation.rs`: Remove `llm_model`/`llm_base_url` match arms, add per-provider equivalents + `llm_provider` validation
- `home.rs`: Default config.toml template → `llm_provider = "anthropic"` + `llm_anthropic_model = "claude-sonnet-4-6"`
- `config.rs` tests: All config cascade tests parse old field names, need rewriting
- `config list` display: Show active provider + model instead of `settings.llm_model`
- `MODEL_ALIASES`: Rethink as `(alias, LlmProvider, model_name)` tuples

## Invariants Preserved

- OAuth token detection (`sk-ant-oat` prefix) stays, scoped to Anthropic key consumption
- `llm_max_tokens` is provider-agnostic — untouched
- `openai_api_key` / `embedding_model` / `embedding_base_url` are for embeddings — untouched
- `cargo clippy` and `cargo test` must pass clean
