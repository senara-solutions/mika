//! Resolution of a one-shot `--model` override — the single site (mika#2304).
//!
//! # Why this lives in `mika-common` and not in the CLI
//!
//! Until mika#2304 these four functions lived in `mika-cli` (`cli.rs` and
//! `init.rs`), which was correct while `mika ask` ran its own agent loop. Since
//! mika#1727 it does not: the turn executes in mika-spirit, reached over A2A, so
//! the provider the model id must be resolved *against* is the executing
//! agent's, not this machine's. On the `--remote` path the two are not even the
//! same deployment.
//!
//! So the raw string travels (`mika.model_override`, see
//! [`mika_a2a::MODEL_OVERRIDE_KEY`]) and the **server** resolves it. `mika-agent`
//! and `mika-cli` share no dependency edge; `mika-common` is where both can
//! reach one implementation. Copying it instead would give two resolvers free to
//! diverge — the class mika#2158 had to engrave after a grooming regex was
//! duplicated and then missed two widenings for months. `mika chat`, still
//! in-process, calls the same functions through the same door.
//!
//! # The ordering inside [`resolve_model_override`] is load-bearing
//!
//! Key check **before** provider construction, not after. `create_provider_with_budget`
//! routes the ten OpenAI-compatible variants — OpenRouter included, which is the
//! rail the founding measurement ran on — to `OpenAiCompatibleProvider::new`,
//! which returns no `Result` and never consults `api_key`. A missing key
//! therefore *succeeds* at construction and fails later as a bare downstream
//! 401. Leaving the refusal to the constructor would write the fail-closed
//! policy in the plan and leave it out of the binary.

use anyhow::Result;

use crate::config::Settings;
use crate::llm::ProviderKind;

/// Known model shorthands: (shorthand, full_model_id, display_name).
///
/// Single source of truth — used by the CLI `--model` flag, the TUI `/model`
/// command, `mika model`, and (since mika#2304) the server-side resolution of a
/// caller-declared override.
pub const MODEL_ALIASES: &[(&str, &str, &str)] = &[
    ("sonnet", "anthropic/claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("opus", "anthropic/claude-opus-4-6", "Claude Opus 4.6"),
    ("haiku", "anthropic/claude-haiku-4-5", "Claude Haiku 4.5"),
    ("gpt4o", "openai/gpt-4o", "GPT-4o"),
    ("deepseek", "deepseek/deepseek-chat", "DeepSeek Chat"),
    ("gemini", "google/gemini-2.5-flash", "Gemini 2.5 Flash"),
];

/// Resolve a model alias (e.g. `"sonnet"`) to its full model id (e.g.
/// `"anthropic/claude-sonnet-4-6"`).
///
/// All aliases carry their provider prefix for cross-provider correctness.
/// Returns the input unchanged when it is not a known alias.
pub fn resolve_model_alias(input: &str) -> String {
    let lower = input.to_lowercase();
    for &(alias, full_id, _display) in MODEL_ALIASES {
        if lower == alias || lower == full_id {
            return full_id.to_string();
        }
    }
    input.to_string()
}

/// Resolve a `--model` override into `(provider, model_id)` for a given
/// configured provider.
///
/// The returned provider is **always** `configured` — the model name's prefix
/// never re-dispatches to a different provider (mika#1591). Aliases are resolved
/// first. A `prefix/rest` model id has its prefix stripped only when `prefix`
/// parses to the configured provider itself; otherwise the full id is preserved
/// (OpenRouter and other vendor-prefixed providers need the full id).
pub fn parse_model_override(model: &str, configured: ProviderKind) -> (ProviderKind, String) {
    let resolved = resolve_model_alias(model);
    if let Some((prefix, rest)) = resolved.split_once('/')
        && let Ok(parsed) = prefix.parse::<ProviderKind>()
        && parsed == configured
    {
        return (configured, rest.to_string());
    }
    (configured, resolved)
}

/// Whether a provider needs an API key to authenticate. Local providers
/// (Ollama, MikaModel — localhost endpoints) do not.
pub fn provider_requires_api_key(provider: ProviderKind) -> bool {
    !matches!(provider, ProviderKind::Ollama | ProviderKind::MikaModel)
}

/// Validate that a key-requiring provider has an API key configured before a
/// `--model` override routes a request to it.
///
/// Returns a named error (provider + model id) instead of letting the request
/// fail with a bare downstream 401 "no API key" (mika#1591 AC2). Since mika#2304
/// this is also the **only** pre-flight refusal the fail-closed policy can rest
/// on: see the module header for why the provider constructor cannot carry it.
pub fn check_provider_key(
    provider: ProviderKind,
    api_key: Option<&str>,
    model_id: &str,
) -> Result<()> {
    if provider_requires_api_key(provider) && api_key.is_none_or(|k| k.trim().is_empty()) {
        anyhow::bail!(
            "Provider '{provider}' has no API key configured. Cannot route model '{model_id}'."
        );
    }
    Ok(())
}

/// The one entry point: turn a raw `--model` string into the pair a provider
/// can be built from, or a named refusal.
///
/// `raw` is the string as the operator typed it. Resolution is, in order:
/// alias → conditional prefix strip against `settings.llm_provider` → API-key
/// check. The order is the whole point — see the module header.
///
/// # What this cannot check
///
/// Whether the provider actually serves that model id. No local validation
/// exists for it at any layer, and acquiring one would mean a network
/// round-trip. An unknown id fails at the first call, on the provider's own
/// 400/404 — which is already fail-closed, and needs no code.
pub fn resolve_model_override(settings: &Settings, raw: &str) -> Result<(ProviderKind, String)> {
    let configured = settings.llm_provider;
    let (provider, model_id) = parse_model_override(raw, configured);
    let (_, api_key, _) = settings.provider_fields(provider);
    check_provider_key(provider, api_key, &model_id)?;
    Ok((provider, model_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The tests below moved verbatim from `mika-cli/src/init.rs` with mika#2304.
    // That they still pass unchanged is what attests the move is a *move* and not
    // a rewrite — `mika chat` reads the same answers it did before (T8).

    // U1 / AC1: a vendor-prefixed id whose prefix does NOT name the configured
    // provider keeps the full id and inherits the configured provider (no
    // prefix re-dispatch). OpenRouter ids are vendor-prefixed.
    #[test]
    fn parse_keeps_full_id_for_openrouter() {
        let (provider, model) = parse_model_override("qwen/qwen3.7-max", ProviderKind::OpenRouter);
        assert_eq!(provider, ProviderKind::OpenRouter);
        assert_eq!(model, "qwen/qwen3.7-max");
    }

    // U1 / AC3: a vendor prefix that matches the configured provider is stripped.
    #[test]
    fn parse_strips_matching_prefix_for_native_qwen() {
        let (provider, model) = parse_model_override("qwen/qwen3.7-max", ProviderKind::Qwen);
        assert_eq!(provider, ProviderKind::Qwen);
        assert_eq!(model, "qwen3.7-max");
    }

    // U1 / AC4: a non-prefixed id is passed through unchanged under its provider.
    #[test]
    fn parse_passes_through_unprefixed_id() {
        let (provider, model) =
            parse_model_override("claude-sonnet-4-6-20250514", ProviderKind::Anthropic);
        assert_eq!(provider, ProviderKind::Anthropic);
        assert_eq!(model, "claude-sonnet-4-6-20250514");
    }

    // U1: aliases resolve before routing and still inherit the configured provider.
    // Under the alias's own native provider the resolved prefix is stripped; under
    // any other provider the full vendor-prefixed id is preserved.
    #[test]
    fn parse_resolves_alias_and_inherits_provider() {
        // "sonnet" resolves to "anthropic/claude-sonnet-4-6"; under Anthropic the
        // matching prefix is stripped to the native id.
        let (provider, model) = parse_model_override("sonnet", ProviderKind::Anthropic);
        assert_eq!(provider, ProviderKind::Anthropic);
        assert_eq!(model, "claude-sonnet-4-6");
        // Under a non-matching provider the resolved full id is kept (OpenRouter ids
        // are vendor-prefixed) and the provider is still inherited.
        let (provider, model) = parse_model_override("sonnet", ProviderKind::OpenRouter);
        assert_eq!(provider, ProviderKind::OpenRouter);
        assert_eq!(model, "anthropic/claude-sonnet-4-6");
    }

    // U1: a non-matching native prefix under a third provider never re-dispatches
    // — guards against regression of the old prefix-routing behavior.
    #[test]
    fn parse_never_redispatches_to_named_native_provider() {
        let (provider, model) = parse_model_override("qwen/qwen3.7-max", ProviderKind::DeepSeek);
        assert_eq!(provider, ProviderKind::DeepSeek);
        assert_eq!(model, "qwen/qwen3.7-max");
    }

    // U1: degenerate model strings never panic and inherit the configured provider.
    // A bare prefix or trailing/leading slash whose prefix is not a provider keeps
    // the full string; an empty string passes through unchanged.
    #[test]
    fn parse_handles_degenerate_strings() {
        assert_eq!(
            parse_model_override("", ProviderKind::Anthropic),
            (ProviderKind::Anthropic, String::new())
        );
        // "foo/" — prefix "foo" is not a provider → full string kept.
        assert_eq!(
            parse_model_override("foo/", ProviderKind::OpenRouter),
            (ProviderKind::OpenRouter, "foo/".to_string())
        );
        // "/bar" — empty prefix is not a provider → full string kept.
        assert_eq!(
            parse_model_override("/bar", ProviderKind::OpenRouter),
            (ProviderKind::OpenRouter, "/bar".to_string())
        );
    }

    // U2 / AC2: a key-requiring provider with no key yields a named error that
    // includes both the provider and the model id.
    #[test]
    fn check_key_errors_name_provider_and_model() {
        let err = check_provider_key(ProviderKind::OpenRouter, None, "qwen/qwen3.7-max")
            .unwrap_err()
            .to_string();
        assert!(err.contains("openrouter"), "error names provider: {err}");
        assert!(err.contains("qwen/qwen3.7-max"), "error names model: {err}");
    }

    // U2 / AC2: an empty/whitespace key is treated as absent.
    #[test]
    fn check_key_treats_blank_key_as_absent() {
        assert!(check_provider_key(ProviderKind::OpenRouter, Some("   "), "m").is_err());
    }

    // U2: a configured key passes the check.
    #[test]
    fn check_key_passes_with_configured_key() {
        assert!(check_provider_key(ProviderKind::OpenRouter, Some("sk-or-xxx"), "m").is_ok());
    }

    // U2: local providers (Ollama, MikaModel) are exempt from the key check.
    #[test]
    fn check_key_exempts_local_providers() {
        assert!(check_provider_key(ProviderKind::Ollama, None, "llama3").is_ok());
        assert!(check_provider_key(ProviderKind::MikaModel, None, "mika").is_ok());
    }

    // --- mika#2304: the single entry point ------------------------------------

    fn openrouter_settings(key: Option<&str>) -> Settings {
        let mut settings = Settings::test_defaults();
        settings.llm_provider = ProviderKind::OpenRouter;
        settings.openrouter_api_key = key.map(secrecy::SecretString::from);
        settings
    }

    /// AC3 core: a declared override the executing agent cannot serve is
    /// **refused**, and the refusal names both halves the operator needs.
    #[test]
    fn mika2304_an_override_without_a_key_is_refused_by_name() {
        let settings = openrouter_settings(None);
        let err = resolve_model_override(&settings, "moonshotai/kimi-k2.5")
            .expect_err("a provider with no API key cannot serve an override");
        let text = err.to_string();
        assert!(text.contains("openrouter"), "names the provider: {text}");
        assert!(
            text.contains("moonshotai/kimi-k2.5"),
            "names the model: {text}"
        );
    }

    /// The entry point composes the same two steps in the same order as the
    /// pieces it replaced — resolution first, key check against the *resolved*
    /// provider second.
    #[test]
    fn mika2304_entry_point_resolves_then_checks() {
        let settings = openrouter_settings(Some("sk-or-xxx"));
        let (provider, model) = resolve_model_override(&settings, "sonnet").unwrap();
        assert_eq!(provider, ProviderKind::OpenRouter);
        // Under OpenRouter the alias keeps its vendor prefix (mika#1591).
        assert_eq!(model, "anthropic/claude-sonnet-4-6");
    }

    /// A local provider needs no key, so an override under it is never refused
    /// for that reason.
    #[test]
    fn mika2304_local_providers_need_no_key_for_an_override() {
        let mut settings = Settings::test_defaults();
        settings.llm_provider = ProviderKind::Ollama;
        let (provider, model) = resolve_model_override(&settings, "llama3").unwrap();
        assert_eq!(provider, ProviderKind::Ollama);
        assert_eq!(model, "llama3");
    }
}
