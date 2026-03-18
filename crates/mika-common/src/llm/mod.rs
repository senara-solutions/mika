pub mod anthropic;
pub mod error;
pub mod openai;
pub mod types;

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;

pub use error::LlmError;
pub use types::*;

/// Provider-agnostic trait for LLM chat completions.
///
/// Each provider (Anthropic, OpenAI-compatible, etc.) implements this trait
/// to translate between provider-agnostic types and their wire format.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a message and return the complete response.
    async fn send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>;

    /// Human-readable provider name (e.g., "anthropic", "openai").
    fn provider_name(&self) -> &str;

    /// The model identifier being used.
    fn model_name(&self) -> &str;

    /// Maximum output tokens configured for this provider.
    fn max_tokens(&self) -> u32;

    /// Whether this provider supports tool/function calling.
    fn supports_tool_calling(&self) -> bool {
        true
    }

    /// Whether this provider supports image/vision inputs.
    fn supports_vision(&self) -> bool {
        false
    }

    /// Whether this provider supports extended thinking/reasoning.
    fn supports_extended_thinking(&self) -> bool {
        false
    }

    /// Perform a lightweight health check (e.g., minimal API call or endpoint ping).
    async fn check_health(&self) -> Result<(), LlmError>;
}

// -- Model spec parsing --

/// Parsed model specification from a model string like `anthropic/claude-sonnet-4-6`.
#[derive(Clone)]
pub struct ModelSpec {
    pub provider: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

impl std::fmt::Debug for ModelSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelSpec")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Known LLM provider kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Ollama,
    Groq,
    /// Generic OpenAI-compatible provider with a custom base URL.
    OpenAiCompatible,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::Anthropic => write!(f, "anthropic"),
            ProviderKind::OpenAi => write!(f, "openai"),
            ProviderKind::Ollama => write!(f, "ollama"),
            ProviderKind::Groq => write!(f, "groq"),
            ProviderKind::OpenAiCompatible => write!(f, "openai-compatible"),
        }
    }
}

impl ModelSpec {
    /// Parse a model string into a `ModelSpec`.
    ///
    /// Format: `provider/model-name` where `provider` is one of the known prefixes.
    /// If no slash is present, defaults to `anthropic/` prefix for backward compatibility.
    ///
    /// Only the first slash is treated as the separator — model names may contain
    /// additional slashes (e.g., `ollama/meta-llama/llama-3.1-8b`).
    pub fn parse(model_string: &str) -> Result<Self> {
        let model_string = model_string.trim();
        if model_string.is_empty() {
            bail!("model string cannot be empty");
        }

        let (provider, model) = if let Some(slash_pos) = model_string.find('/') {
            let prefix = &model_string[..slash_pos];
            let model = &model_string[slash_pos + 1..];
            if model.is_empty() {
                bail!("model name cannot be empty after provider prefix '{prefix}/'");
            }
            let provider = match prefix {
                "anthropic" => ProviderKind::Anthropic,
                "openai" => ProviderKind::OpenAi,
                "ollama" => ProviderKind::Ollama,
                "groq" => ProviderKind::Groq,
                "openai-compatible" => ProviderKind::OpenAiCompatible,
                _ => bail!(
                    "unknown provider prefix '{prefix}'. \
                     Known prefixes: anthropic, openai, ollama, groq, openai-compatible"
                ),
            };
            (provider, model.to_string())
        } else {
            // No prefix — default to Anthropic for backward compatibility
            (ProviderKind::Anthropic, model_string.to_string())
        };

        Ok(Self {
            provider,
            model,
            base_url: None,
            api_key: None,
        })
    }

    /// Set the base URL override.
    pub fn with_base_url(mut self, url: Option<String>) -> Self {
        self.base_url = url;
        self
    }

    /// Set the API key.
    pub fn with_api_key(mut self, key: Option<String>) -> Self {
        self.api_key = key;
        self
    }

    /// Get the default base URL for this provider kind.
    pub fn default_base_url(&self) -> Option<&'static str> {
        match self.provider {
            ProviderKind::Anthropic => Some("https://api.anthropic.com"),
            ProviderKind::OpenAi => Some("https://api.openai.com/v1"),
            ProviderKind::Ollama => Some("http://localhost:11434/v1"),
            ProviderKind::Groq => Some("https://api.groq.com/openai/v1"),
            ProviderKind::OpenAiCompatible => None, // requires explicit base_url
        }
    }

    /// Resolve the effective base URL (explicit override > default).
    pub fn effective_base_url(&self) -> Option<String> {
        self.base_url
            .clone()
            .or_else(|| self.default_base_url().map(String::from))
    }
}

/// Create an `LlmProvider` from a `ModelSpec` and max_tokens setting.
///
/// For Anthropic, `api_key` is resolved from `spec.api_key`.
/// For OpenAI-compatible providers, `base_url` and `api_key` are resolved from spec.
pub fn create_provider(spec: &ModelSpec, max_tokens: u32) -> Result<Arc<dyn LlmProvider>> {
    match spec.provider {
        ProviderKind::Anthropic => {
            let provider = anthropic::AnthropicProvider::new(
                spec.api_key.clone(),
                spec.model.clone(),
                max_tokens,
            )
            .context("failed to create Anthropic provider")?;
            Ok(Arc::new(provider))
        }
        ProviderKind::OpenAi
        | ProviderKind::Ollama
        | ProviderKind::Groq
        | ProviderKind::OpenAiCompatible => {
            if matches!(spec.provider, ProviderKind::OpenAiCompatible) {
                tracing::warn!(
                    base_url = ?spec.base_url,
                    "MIKA_LLM_API_KEY will be sent to custom endpoint; ensure you trust this URL"
                );
            }
            let base_url = spec.effective_base_url().ok_or_else(|| {
                anyhow::anyhow!(
                    "base URL is required for provider '{}'. Set MIKA_LLM_BASE_URL.",
                    spec.provider
                )
            })?;
            let provider = openai::OpenAiCompatibleProvider::new(
                base_url,
                spec.api_key.clone(),
                spec.model.clone(),
                max_tokens,
                spec.provider,
            );
            Ok(Arc::new(provider))
        }
    }
}

/// Create a no-op provider that cannot make API calls.
/// Used as a placeholder in contexts where the provider is required
/// but will never be called (e.g., team mode TUI).
pub fn dummy_provider() -> Arc<dyn LlmProvider> {
    Arc::new(anthropic::AnthropicProvider::dummy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_anthropic_prefix() {
        let spec = ModelSpec::parse("anthropic/claude-sonnet-4-6").unwrap();
        assert_eq!(spec.provider, ProviderKind::Anthropic);
        assert_eq!(spec.model, "claude-sonnet-4-6");
    }

    #[test]
    fn test_parse_openai_prefix() {
        let spec = ModelSpec::parse("openai/gpt-4o").unwrap();
        assert_eq!(spec.provider, ProviderKind::OpenAi);
        assert_eq!(spec.model, "gpt-4o");
    }

    #[test]
    fn test_parse_ollama_prefix() {
        let spec = ModelSpec::parse("ollama/llama3").unwrap();
        assert_eq!(spec.provider, ProviderKind::Ollama);
        assert_eq!(spec.model, "llama3");
    }

    #[test]
    fn test_parse_groq_prefix() {
        let spec = ModelSpec::parse("groq/llama-3.1-70b").unwrap();
        assert_eq!(spec.provider, ProviderKind::Groq);
        assert_eq!(spec.model, "llama-3.1-70b");
    }

    #[test]
    fn test_parse_openai_compatible() {
        let spec = ModelSpec::parse("openai-compatible/my-model").unwrap();
        assert_eq!(spec.provider, ProviderKind::OpenAiCompatible);
        assert_eq!(spec.model, "my-model");
    }

    #[test]
    fn test_parse_no_prefix_defaults_to_anthropic() {
        let spec = ModelSpec::parse("claude-sonnet-4-6").unwrap();
        assert_eq!(spec.provider, ProviderKind::Anthropic);
        assert_eq!(spec.model, "claude-sonnet-4-6");
    }

    #[test]
    fn test_parse_model_with_slashes() {
        let spec = ModelSpec::parse("ollama/meta-llama/llama-3.1-8b").unwrap();
        assert_eq!(spec.provider, ProviderKind::Ollama);
        assert_eq!(spec.model, "meta-llama/llama-3.1-8b");
    }

    #[test]
    fn test_parse_empty_string_fails() {
        assert!(ModelSpec::parse("").is_err());
        assert!(ModelSpec::parse("   ").is_err());
    }

    #[test]
    fn test_parse_empty_model_after_prefix_fails() {
        assert!(ModelSpec::parse("anthropic/").is_err());
    }

    #[test]
    fn test_parse_unknown_prefix_fails() {
        let err = ModelSpec::parse("unknown/model").unwrap_err();
        assert!(err.to_string().contains("unknown provider prefix"));
    }

    #[test]
    fn test_default_base_urls() {
        let spec = ModelSpec::parse("ollama/llama3").unwrap();
        assert_eq!(spec.default_base_url(), Some("http://localhost:11434/v1"));

        let spec = ModelSpec::parse("openai-compatible/model").unwrap();
        assert_eq!(spec.default_base_url(), None);
    }

    #[test]
    fn test_effective_base_url_override() {
        let spec = ModelSpec::parse("ollama/llama3")
            .unwrap()
            .with_base_url(Some("http://custom:1234/v1".into()));
        assert_eq!(
            spec.effective_base_url(),
            Some("http://custom:1234/v1".into())
        );
    }

    #[test]
    fn test_provider_kind_display() {
        assert_eq!(ProviderKind::Anthropic.to_string(), "anthropic");
        assert_eq!(ProviderKind::OpenAi.to_string(), "openai");
        assert_eq!(ProviderKind::Ollama.to_string(), "ollama");
        assert_eq!(
            ProviderKind::OpenAiCompatible.to_string(),
            "openai-compatible"
        );
    }
}
