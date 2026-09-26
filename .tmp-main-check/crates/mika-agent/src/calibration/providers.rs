//! Provider construction helpers for the calibration framework.
//!
//! Parses provider/model specifications and constructs real LLM providers.
//! Used by both the `calibrate` binary and the eval test matrix.

use std::str::FromStr;
use std::sync::Arc;

use mika_common::llm::{LlmProvider, ModelSpec, ProviderKind, create_provider};

/// Parse a comma-separated provider list string.
///
/// Returns a list of `ProviderKind` values. Special value `all` returns all providers.
/// Empty input returns an empty vec. Unknown names panic with a listing of valid providers.
pub fn parse_provider_list(input: &str) -> Vec<ProviderKind> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.eq_ignore_ascii_case("all") {
        return ProviderKind::ALL.to_vec();
    }

    trimmed
        .split(',')
        .map(|s| {
            let s = s.trim();
            ProviderKind::from_str(s).unwrap_or_else(|_| {
                let valid: Vec<&str> = ProviderKind::ALL
                    .iter()
                    .map(|p| p.config_prefix())
                    .collect();
                panic!(
                    "Unknown provider '{}'. Valid providers: {}",
                    s,
                    valid.join(", ")
                );
            })
        })
        .collect()
}

/// Construct a real LLM provider from environment variables.
///
/// Returns `None` when the API key for the given provider is not configured,
/// allowing callers to skip that provider gracefully.
pub fn create_real_provider(kind: ProviderKind) -> Option<Arc<dyn LlmProvider>> {
    let prefix = kind.config_prefix().to_uppercase();
    let env_key = format!("MIKA_{}_API_KEY", prefix);
    let api_key = std::env::var(&env_key).ok();

    // For Ollama, API key is usually not needed
    if api_key.is_none() && kind != ProviderKind::Ollama {
        return None;
    }

    let spec = ModelSpec {
        provider: kind,
        model: kind.default_model().to_string(),
        base_url: kind.default_base_url().map(|s| s.to_string()),
        api_key,
    };

    match create_provider(&spec, kind.max_output_tokens(), false) {
        Ok(provider) => Some(provider),
        Err(e) => {
            eprintln!(
                "Warning: failed to create {} provider: {}. Skipping.",
                kind, e
            );
            None
        }
    }
}

/// Construct a real LLM provider from a `provider/model` specification string.
///
/// Format: `provider_name/model_name` (e.g., `anthropic/claude-sonnet-4-6`).
/// Returns `None` when the API key is not configured.
pub fn create_provider_from_spec(spec_str: &str) -> Option<Arc<dyn LlmProvider>> {
    let (provider_str, model_str) = spec_str.split_once('/').unwrap_or_else(|| {
        panic!(
            "Invalid model spec '{}': expected 'provider/model'",
            spec_str
        )
    });

    let kind = ProviderKind::from_str(provider_str).unwrap_or_else(|_| {
        let valid: Vec<&str> = ProviderKind::ALL
            .iter()
            .map(|p| p.config_prefix())
            .collect();
        panic!(
            "Unknown provider '{}'. Valid providers: {}",
            provider_str,
            valid.join(", ")
        );
    });

    let prefix = kind.config_prefix().to_uppercase();
    let env_key = format!("MIKA_{}_API_KEY", prefix);
    let api_key = std::env::var(&env_key).ok();

    if api_key.is_none() && kind != ProviderKind::Ollama {
        return None;
    }

    let spec = ModelSpec {
        provider: kind,
        model: model_str.to_string(),
        base_url: kind.default_base_url().map(|s| s.to_string()),
        api_key,
    };

    match create_provider(&spec, kind.max_output_tokens(), false) {
        Ok(provider) => Some(provider),
        Err(e) => {
            eprintln!(
                "Warning: failed to create {}/{} provider: {}. Skipping.",
                provider_str, model_str, e
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_returns_empty() {
        assert!(parse_provider_list("").is_empty());
    }

    #[test]
    fn test_parse_whitespace_returns_empty() {
        assert!(parse_provider_list("  ").is_empty());
    }

    #[test]
    fn test_parse_all_returns_all_providers() {
        assert_eq!(parse_provider_list("all").len(), ProviderKind::ALL.len());
    }

    #[test]
    fn test_parse_all_case_insensitive() {
        assert_eq!(parse_provider_list("ALL").len(), ProviderKind::ALL.len());
        assert_eq!(parse_provider_list("All").len(), ProviderKind::ALL.len());
    }

    #[test]
    fn test_parse_comma_separated() {
        let result = parse_provider_list("anthropic,openai");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ProviderKind::Anthropic);
        assert_eq!(result[1], ProviderKind::OpenAi);
    }

    #[test]
    fn test_parse_case_insensitive() {
        let result = parse_provider_list("Anthropic,OPENAI,groq");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], ProviderKind::Anthropic);
        assert_eq!(result[1], ProviderKind::OpenAi);
        assert_eq!(result[2], ProviderKind::Groq);
    }

    #[test]
    fn test_parse_with_whitespace() {
        let result = parse_provider_list(" anthropic , openai ");
        assert_eq!(result.len(), 2);
    }

    #[test]
    #[should_panic(expected = "Unknown provider 'foobar'")]
    fn test_parse_unknown_provider_panics() {
        parse_provider_list("anthropic,foobar");
    }

    #[test]
    fn test_create_real_provider_no_key_returns_none() {
        // Groq key is likely not set in test environment
        assert!(create_real_provider(ProviderKind::Groq).is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_oauth_token_creates_provider() {
        // Set an OAuth-prefix key for this test only
        unsafe { std::env::set_var("MIKA_ANTHROPIC_API_KEY", "sk-ant-oat01-test-token-dummy") };
        let provider = create_provider_from_spec("anthropic/claude-sonnet-4-6");
        assert!(
            provider.is_some(),
            "OAuth token should create a valid provider"
        );
        unsafe { std::env::remove_var("MIKA_ANTHROPIC_API_KEY") };
    }
}
