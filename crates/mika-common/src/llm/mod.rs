pub mod anthropic;
pub mod error;
pub mod openai;
pub mod types;

use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

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

// -- Model spec --

/// Resolved model specification for creating an LLM provider.
///
/// Constructed from per-provider `Settings` fields by `Settings::make_llm_provider()`.
/// Not parsed from a string — the provider/model/base_url/api_key are all explicit.
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

impl ModelSpec {
    /// Resolve the effective base URL (explicit override > provider default).
    pub fn effective_base_url(&self) -> Option<String> {
        self.base_url
            .clone()
            .or_else(|| self.provider.default_base_url().map(String::from))
    }
}

// -- Provider kinds --

/// All known LLM provider kinds.
///
/// Each variant maps to a config key prefix (e.g., `anthropic_model`, `openai_api_key`),
/// a default base URL, and a provider implementation (Anthropic-native or OpenAI-compatible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "openrouter")]
    OpenRouter,
    Groq,
    Ollama,
    Mistral,
    Google,
    #[serde(rename = "deepseek")]
    DeepSeek,
    #[serde(rename = "minimax")]
    MiniMax,
    Kimi,
    Qwen,
}

impl ProviderKind {
    /// All provider kinds in canonical order.
    pub const ALL: &[ProviderKind] = &[
        ProviderKind::Anthropic,
        ProviderKind::OpenAi,
        ProviderKind::OpenRouter,
        ProviderKind::Groq,
        ProviderKind::Ollama,
        ProviderKind::Mistral,
        ProviderKind::Google,
        ProviderKind::DeepSeek,
        ProviderKind::MiniMax,
        ProviderKind::Kimi,
        ProviderKind::Qwen,
    ];

    /// Config key prefix for per-provider settings (e.g., `"anthropic"` → `anthropic_model`).
    pub fn config_prefix(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::OpenRouter => "openrouter",
            ProviderKind::Groq => "groq",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Mistral => "mistral",
            ProviderKind::Google => "google",
            ProviderKind::DeepSeek => "deepseek",
            ProviderKind::MiniMax => "minimax",
            ProviderKind::Kimi => "kimi",
            ProviderKind::Qwen => "qwen",
        }
    }

    /// Default base URL for this provider. All providers have a built-in default.
    pub fn default_base_url(&self) -> Option<&'static str> {
        match self {
            ProviderKind::Anthropic => Some("https://api.anthropic.com"),
            ProviderKind::OpenAi => Some("https://api.openai.com/v1"),
            ProviderKind::OpenRouter => Some("https://openrouter.ai/api/v1"),
            ProviderKind::Groq => Some("https://api.groq.com/openai/v1"),
            ProviderKind::Ollama => Some("http://localhost:11434/v1"),
            ProviderKind::Mistral => Some("https://api.mistral.ai/v1"),
            ProviderKind::Google => Some("https://generativelanguage.googleapis.com/v1beta/openai"),
            ProviderKind::DeepSeek => Some("https://api.deepseek.com"),
            ProviderKind::MiniMax => Some("https://api.minimax.io/v1"),
            ProviderKind::Kimi => Some("https://api.moonshot.cn/v1"),
            ProviderKind::Qwen => Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        }
    }

    /// Default model for this provider (used when no model is explicitly configured).
    pub fn default_model(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "claude-sonnet-4-6",
            ProviderKind::OpenAi => "gpt-4o",
            ProviderKind::OpenRouter => "anthropic/claude-sonnet-4",
            ProviderKind::Groq => "llama-3.3-70b-versatile",
            ProviderKind::Ollama => "llama3",
            ProviderKind::Mistral => "mistral-large-latest",
            ProviderKind::Google => "gemini-2.5-flash",
            ProviderKind::DeepSeek => "deepseek-chat",
            ProviderKind::MiniMax => "MiniMax-M2.7",
            ProviderKind::Kimi => "moonshot-v1-128k",
            ProviderKind::Qwen => "qwen-plus",
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.config_prefix())
    }
}

impl FromStr for ProviderKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "anthropic" => Ok(ProviderKind::Anthropic),
            "openai" => Ok(ProviderKind::OpenAi),
            "openrouter" => Ok(ProviderKind::OpenRouter),
            "groq" => Ok(ProviderKind::Groq),
            "ollama" => Ok(ProviderKind::Ollama),
            "mistral" => Ok(ProviderKind::Mistral),
            "google" => Ok(ProviderKind::Google),
            "deepseek" => Ok(ProviderKind::DeepSeek),
            "minimax" => Ok(ProviderKind::MiniMax),
            "kimi" => Ok(ProviderKind::Kimi),
            "qwen" => Ok(ProviderKind::Qwen),
            _ => Err(format!(
                "unknown provider '{s}'. Known providers: anthropic, openai, openrouter, groq, ollama, mistral, google, deepseek, minimax, kimi, qwen"
            )),
        }
    }
}

/// Create an `LlmProvider` from a `ModelSpec` and max_tokens setting.
///
/// For Anthropic, uses the native Anthropic API client.
/// For all others, uses the OpenAI-compatible provider.
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
        _ => {
            let base_url = spec.effective_base_url().ok_or_else(|| {
                anyhow::anyhow!(
                    "base URL is required for provider '{}'. Set {}_base_url in config.toml.",
                    spec.provider,
                    spec.provider.config_prefix()
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
    fn test_provider_kind_display() {
        assert_eq!(ProviderKind::Anthropic.to_string(), "anthropic");
        assert_eq!(ProviderKind::OpenAi.to_string(), "openai");
        assert_eq!(ProviderKind::OpenRouter.to_string(), "openrouter");
        assert_eq!(ProviderKind::Groq.to_string(), "groq");
        assert_eq!(ProviderKind::Ollama.to_string(), "ollama");
        assert_eq!(ProviderKind::Mistral.to_string(), "mistral");
        assert_eq!(ProviderKind::Google.to_string(), "google");
        assert_eq!(ProviderKind::DeepSeek.to_string(), "deepseek");
    }

    #[test]
    fn test_provider_kind_from_str() {
        assert_eq!(
            "anthropic".parse::<ProviderKind>(),
            Ok(ProviderKind::Anthropic)
        );
        assert_eq!("openai".parse::<ProviderKind>(), Ok(ProviderKind::OpenAi));
        assert_eq!(
            "openrouter".parse::<ProviderKind>(),
            Ok(ProviderKind::OpenRouter)
        );
        assert_eq!("groq".parse::<ProviderKind>(), Ok(ProviderKind::Groq));
        assert_eq!("ollama".parse::<ProviderKind>(), Ok(ProviderKind::Ollama));
        assert_eq!("mistral".parse::<ProviderKind>(), Ok(ProviderKind::Mistral));
        assert_eq!("google".parse::<ProviderKind>(), Ok(ProviderKind::Google));
        assert_eq!(
            "deepseek".parse::<ProviderKind>(),
            Ok(ProviderKind::DeepSeek)
        );
    }

    #[test]
    fn test_provider_kind_from_str_case_insensitive() {
        assert_eq!(
            "Anthropic".parse::<ProviderKind>(),
            Ok(ProviderKind::Anthropic)
        );
        assert_eq!("OPENAI".parse::<ProviderKind>(), Ok(ProviderKind::OpenAi));
        assert_eq!(
            "DeepSeek".parse::<ProviderKind>(),
            Ok(ProviderKind::DeepSeek)
        );
    }

    #[test]
    fn test_provider_kind_from_str_unknown() {
        let err = "unknown".parse::<ProviderKind>().unwrap_err();
        assert!(err.contains("unknown provider"));
        assert!(err.contains("anthropic"));
        assert!(err.contains("deepseek"));
    }

    #[test]
    fn test_provider_kind_deserialize() {
        #[derive(Deserialize)]
        struct TestConfig {
            provider: ProviderKind,
        }
        let config: TestConfig = toml::from_str(r#"provider = "anthropic""#).unwrap();
        assert_eq!(config.provider, ProviderKind::Anthropic);

        let config: TestConfig = toml::from_str(r#"provider = "deepseek""#).unwrap();
        assert_eq!(config.provider, ProviderKind::DeepSeek);

        let config: TestConfig = toml::from_str(r#"provider = "openrouter""#).unwrap();
        assert_eq!(config.provider, ProviderKind::OpenRouter);
    }

    #[test]
    fn test_provider_kind_all() {
        assert_eq!(ProviderKind::ALL.len(), 8);
        assert_eq!(ProviderKind::ALL[0], ProviderKind::Anthropic);
        assert_eq!(ProviderKind::ALL[7], ProviderKind::DeepSeek);
    }

    #[test]
    fn test_default_base_urls() {
        assert_eq!(
            ProviderKind::Anthropic.default_base_url(),
            Some("https://api.anthropic.com")
        );
        assert_eq!(
            ProviderKind::OpenAi.default_base_url(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            ProviderKind::OpenRouter.default_base_url(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(
            ProviderKind::Groq.default_base_url(),
            Some("https://api.groq.com/openai/v1")
        );
        assert_eq!(
            ProviderKind::Ollama.default_base_url(),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(
            ProviderKind::DeepSeek.default_base_url(),
            Some("https://api.deepseek.com")
        );
    }

    #[test]
    fn test_config_prefix() {
        assert_eq!(ProviderKind::Anthropic.config_prefix(), "anthropic");
        assert_eq!(ProviderKind::OpenRouter.config_prefix(), "openrouter");
        assert_eq!(ProviderKind::DeepSeek.config_prefix(), "deepseek");
    }

    #[test]
    fn test_default_model() {
        assert_eq!(ProviderKind::Anthropic.default_model(), "claude-sonnet-4-6");
        assert_eq!(ProviderKind::OpenAi.default_model(), "gpt-4o");
        assert_eq!(ProviderKind::Ollama.default_model(), "llama3");
    }

    #[test]
    fn test_model_spec_effective_base_url_uses_override() {
        let spec = ModelSpec {
            provider: ProviderKind::Ollama,
            model: "llama3".into(),
            base_url: Some("http://custom:1234/v1".into()),
            api_key: None,
        };
        assert_eq!(
            spec.effective_base_url(),
            Some("http://custom:1234/v1".into())
        );
    }

    #[test]
    fn test_model_spec_effective_base_url_uses_default() {
        let spec = ModelSpec {
            provider: ProviderKind::Ollama,
            model: "llama3".into(),
            base_url: None,
            api_key: None,
        };
        assert_eq!(
            spec.effective_base_url(),
            Some("http://localhost:11434/v1".into())
        );
    }
}
