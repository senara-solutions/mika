//! Real-provider env gate and construction helpers for the eval matrix.
//!
//! Controls which LLM providers are tested via the `MIKA_EVAL_REAL_PROVIDERS` env var.
//! Unknown provider names hard-fail to prevent CI from silently testing nothing.

use std::str::FromStr;
use std::sync::Arc;

use mika_common::llm::{LlmProvider, ModelSpec, ProviderKind, create_provider};

/// Parse the `MIKA_EVAL_REAL_PROVIDERS` environment variable.
///
/// Returns a list of `ProviderKind` values. Special value `all` returns all 11 providers.
/// Empty or unset returns an empty vec. Unknown names panic with a listing of valid providers.
pub fn parse_real_providers() -> Vec<ProviderKind> {
    let val = match std::env::var("MIKA_EVAL_REAL_PROVIDERS") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Vec::new(),
    };
    parse_provider_list(&val)
}

/// Parse a comma-separated provider list string.
///
/// Extracted from `parse_real_providers` for testability without env var mutation.
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
                    "MIKA_EVAL_REAL_PROVIDERS: unknown provider '{}'. Valid providers: {}",
                    s,
                    valid.join(", ")
                );
            })
        })
        .collect()
}

/// Check if any real providers are configured for testing.
pub fn has_real_providers() -> bool {
    !parse_real_providers().is_empty()
}

/// Construct a real LLM provider from environment variables.
///
/// Returns `None` when the API key for the given provider is not configured,
/// allowing tests to skip that provider gracefully.
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
    #[should_panic(expected = "unknown provider 'foobar'")]
    fn test_parse_unknown_provider_panics() {
        parse_provider_list("anthropic,foobar");
    }

    #[test]
    fn test_create_real_provider_no_key_returns_none() {
        // Groq key is likely not set in test environment
        assert!(create_real_provider(ProviderKind::Groq).is_none());
    }
}
