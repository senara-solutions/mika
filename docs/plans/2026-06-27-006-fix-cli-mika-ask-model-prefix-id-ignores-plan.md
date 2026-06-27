# Plan: fix(cli): `mika ask --model <prefix>/<id>` ignores agent's configured llm_provider

**Issue:** mika#1591
**Type:** bug fix
**Branch:** `fix/1591/cli-mika-ask-model-prefix-id-ignores`

## Problem

`AppContext::override_model()` in `crates/mika-cli/src/init.rs:83-107` unconditionally prefix-parses slashed model strings (e.g., `qwen/qwen3.7-max`) and re-dispatches to the native provider matching the prefix. This ignores the agent's configured `llm_provider` — when the agent is configured for OpenRouter, the `qwen/` prefix triggers native Qwen routing (which has no API key), producing a 401.

The TUI's `/model` command already handles this correctly via `parse_provider_model()` in `crates/mika-cli/src/commands/model.rs:148-160`: when `default_provider.model_names_contain_slash()` returns true (OpenRouter), the entire input is treated as a model name with no prefix splitting.

## Root Cause

`override_model()` lacks the `model_names_contain_slash()` guard that `parse_provider_model()` uses. The fix is to align `override_model()` with the TUI's existing logic.

## Implementation

### Step 1: Fix `override_model()` in `crates/mika-cli/src/init.rs`

**What:** Add the `model_names_contain_slash()` guard before prefix-parsing.

**Before (lines 83-107):**
```rust
pub fn override_model(&mut self, model: &str) -> Result<()> {
    let resolved = crate::cli::resolve_model_alias(model);
    if let Some(slash_pos) = resolved.find('/') {
        let prefix = &resolved[..slash_pos];
        if let Ok(provider) = prefix.parse::<mika_common::llm::ProviderKind>() {
            let model_name = resolved[slash_pos + 1..].to_string();
            self.db_ctx.settings.llm_provider = provider;
            self.db_ctx.settings.set_provider_model(provider, Some(model_name));
            self.llm = self.db_ctx.settings.make_llm_provider()?;
            return Ok(());
        }
    }
    let provider = self.db_ctx.settings.llm_provider;
    self.db_ctx.settings.set_provider_model(provider, Some(resolved));
    self.llm = self.db_ctx.settings.make_llm_provider()?;
    Ok(())
}
```

**After:**
```rust
pub fn override_model(&mut self, model: &str) -> Result<()> {
    let resolved = crate::cli::resolve_model_alias(model);
    let current_provider = self.db_ctx.settings.llm_provider;

    // If current provider uses slashes in model names (e.g., OpenRouter uses
    // "qwen/qwen-plus"), don't interpret the slash as a cross-provider switch.
    // This matches the TUI's parse_provider_model() logic in commands/model.rs.
    if !current_provider.model_names_contain_slash() {
        if let Some(slash_pos) = resolved.find('/') {
            let prefix = &resolved[..slash_pos];
            if let Ok(provider) = prefix.parse::<mika_common::llm::ProviderKind>() {
                let model_name = resolved[slash_pos + 1..].to_string();
                self.db_ctx.settings.llm_provider = provider;
                self.db_ctx.settings.set_provider_model(provider, Some(model_name));
                self.llm = self.db_ctx.settings.make_llm_provider()?;
                return Ok(());
            }
        }
    }

    // Plain model name or slash-in-name provider — override the active provider's model
    self.db_ctx.settings.set_provider_model(current_provider, Some(resolved));
    self.llm = self.db_ctx.settings.make_llm_provider()?;
    Ok(())
}
```

**Covers:** AC1, AC3, AC4.

- AC1: `mika ask --model qwen/qwen3.7-max` with `llm_provider = "openrouter"` — OpenRouter uses slashes, so the guard fires, the full string `qwen/qwen3.7-max` is passed as the model to OpenRouter.
- AC3: `mika ask --model qwen/qwen3.7-max` with `llm_provider = "qwen"` — Qwen does NOT use slashes, so prefix-parsing fires, detects `qwen` as a valid provider which matches the configured provider, strips the prefix, and sets model to `qwen3.7-max` on native Qwen. (Note: if the Qwen API requires the prefixed form, the full string is already passed — the prefix-stripping only occurs when the prefix matches a known provider. The AC3 text says "strips the prefix when it matches the configured provider name" — this is the natural behavior of the existing prefix-parse path.)
- AC4: `mika ask --model claude-sonnet-4-6-20250514` with `llm_provider = "anthropic"` — no slash in model string, falls through to plain-model path unchanged.

### Step 2: Improve error message on provider-key-absent failure (AC2)

**What:** Add a pre-flight API key check after provider resolution in `override_model()`, before calling `make_llm_provider()`.

**Where:** `crates/mika-cli/src/init.rs`, inside `override_model()`, after the provider is determined and model is set, before `make_llm_provider()`.

**Logic:** After setting the provider and model, check whether the resolved provider has an API key configured. If not (and the provider requires one — all except Ollama and MikaModel which use base URLs), bail with a descriptive error:

```rust
// Pre-flight: check API key before building the provider
let resolved_provider = self.db_ctx.settings.llm_provider;
let (_, api_key, _) = self.db_ctx.settings.provider_fields(resolved_provider);
let needs_key = !matches!(resolved_provider,
    mika_common::llm::ProviderKind::Ollama | mika_common::llm::ProviderKind::MikaModel
);
if needs_key && api_key.is_none() {
    let model_display = self.db_ctx.settings.active_llm_config().model;
    anyhow::bail!(
        "Provider '{}' has no API key configured. Cannot route model '{}'.",
        resolved_provider, model_display
    );
}
```

This check runs on both code paths (prefix-parsed provider switch and plain model override), so it catches:
- A prefix-parsed switch to a provider with no key (when current provider does NOT use slashes)
- The configured provider itself having no key

### Step 3: Update `--model` help text (AC5)

**Where:** `crates/mika-cli/src/cli.rs`, `AskArgs.model` field doc comment (line 218).

**Before:**
```rust
/// LLM model override for this invocation (e.g., sonnet, opus, haiku, openai/gpt-4o).
/// One-shot override, not persisted to config.
```

**After:**
```rust
/// LLM model override for this invocation (e.g., sonnet, opus, haiku, openai/gpt-4o).
/// Overrides the model id for this request. Routes through the agent's configured
/// llm_provider — does not re-dispatch based on model name prefix.
/// One-shot override, not persisted to config.
```

### Step 4: Update `crates/mika-cli/CLAUDE.md` (AC6)

**Where:** `crates/mika-cli/CLAUDE.md`, `mika ask` section, near the `--model` flag description.

**Add note:** After the existing `--model <model>` description, add: "`--model` is a model-id-only override; the provider is always inherited from agent config. When the agent's provider uses slash-separated model names (e.g., OpenRouter `qwen/qwen-plus`), the slash is NOT interpreted as a provider switch."

### Step 5: Unit test (AC7)

**Where:** `crates/mika-cli/src/init.rs`, inline `#[cfg(test)] mod tests`.

**Tests:**

1. **`test_override_model_openrouter_preserves_slashed_model`** — Configure settings with `llm_provider = OpenRouter` + OpenRouter API key. Call `override_model("qwen/qwen3.7-max")`. Assert provider remains `OpenRouter` and model is `qwen/qwen3.7-max`.

2. **`test_override_model_prefix_switches_provider_when_not_slash_provider`** — Configure settings with `llm_provider = Anthropic` + both Anthropic and OpenAI API keys. Call `override_model("openai/gpt-4o")`. Assert provider changed to `OpenAi` and model is `gpt-4o`.

3. **`test_override_model_plain_name_stays_on_current_provider`** — Configure settings with `llm_provider = Anthropic` + Anthropic API key. Call `override_model("claude-sonnet-4-6-20250514")`. Assert provider remains `Anthropic` and model is `claude-sonnet-4-6-20250514`.

4. **`test_override_model_no_key_error_message`** — Configure settings with `llm_provider = Anthropic`, no API key. Call `override_model("some-model")`. Assert error message contains "Provider 'anthropic' has no API key configured" and the model name.

**Implementation note:** These tests need an `AppContext` with real `Settings`. Use `Settings::test_defaults()` and set the relevant fields. The `make_llm_provider()` call inside `override_model()` validates API keys, so the test for case 1 needs a mock or test API key set. Alternatively, test at the `Settings` level by extracting the provider-resolution logic into a pure function (like the TUI's `parse_provider_model`) and testing that directly. The simplest approach: extract a `resolve_model_provider(model: &str, current_provider: ProviderKind) -> (ProviderKind, String)` helper in `init.rs` and test it directly, avoiding the need for a full `AppContext`.

## File Change Summary

| File | Change |
|------|--------|
| `crates/mika-cli/src/init.rs` | Fix `override_model()` with `model_names_contain_slash()` guard + API key pre-flight check + extract `resolve_model_provider()` helper + unit tests |
| `crates/mika-cli/src/cli.rs` | Update `--model` flag help text |
| `crates/mika-cli/CLAUDE.md` | Add `--model` provider-routing note |

## Verification

```bash
cargo test -p mika-cli
cargo clippy -p mika-cli
```

Manual smoke test:
```bash
# AC1: OpenRouter agent with qwen model
mika ask --model qwen/qwen3.7-max "ping"
# Should succeed via OpenRouter (not 401)

# AC4: Anthropic agent with plain model
mika ask --model claude-sonnet-4-6-20250514 "ping"
# Should succeed via Anthropic
```
