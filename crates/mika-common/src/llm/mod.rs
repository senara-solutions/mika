pub mod anthropic;
pub mod error;
#[cfg(any(test, feature = "test-utils"))]
pub mod mock;
pub mod models;
pub mod openai;
pub mod types;

use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;

pub use error::LlmError;
pub use types::*;

/// Internal tag names that should be stripped from LLM response text.
/// These tags are injected into conversation history for LLM context (tool history,
/// callback results, task health, etc.) but the LLM may echo them in its response.
const INTERNAL_TAG_NAMES: &[&str] = &[
    "context",
    "callback_result",
    "task-health",
    "task-health-instructions",
    "active-work-items",
    "anomalies",
    "rewind_reversals",
];

/// Regex to collapse runs of 3+ newlines (left behind after tag removal) into 2.
static BLANK_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n{3,}").expect("blank line regex must compile"));

/// Build a compiled regex for a specific tag name that matches `<tag ...>...</tag>`.
/// Uses `(?s)` for dotall (multi-line content) and `.*?` for lazy matching.
///
/// Closing tag tolerates malformed variants from non-Anthropic models (#453):
///   - Well-formed: `</tag>`
///   - Whitespace: `< /tag>`, `</ tag>`, `< / tag >`
///   - Bare: `tag>` (missing `</`)
fn build_tag_regex(tag: &str) -> Regex {
    Regex::new(&format!(
        r"(?s)<{tag}\b[^>]*>.*?(?:<\s*/\s*{tag}\s*>|{tag}\s*>)"
    ))
    .expect("tag regex must compile")
}

/// Lazily compiled regexes for each internal tag.
static INTERNAL_TAG_RES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    INTERNAL_TAG_NAMES
        .iter()
        .map(|t| build_tag_regex(t))
        .collect()
});

/// Strip internal metadata XML tags from LLM response text.
///
/// These tags are injected into conversation history for LLM context but should
/// never be displayed to users. Returns the cleaned text with excess blank lines
/// collapsed. If the result is empty or whitespace-only after stripping, returns
/// an empty string — callers should convert to `None` as appropriate.
pub fn strip_internal_tags(text: &str) -> String {
    // Fast path: no angle bracket means no XML tags possible.
    // Most LLM responses contain no internal tags, so this avoids
    // regex evaluation entirely in the common case.
    if !text.contains('<') {
        return text.trim().to_string();
    }

    let mut result = text.to_string();
    for re in INTERNAL_TAG_RES.iter() {
        result = re.replace_all(&result, "").into_owned();
    }
    let collapsed = BLANK_LINE_RE.replace_all(&result, "\n\n");
    collapsed.trim().to_string()
}

/// Provider-agnostic trait for LLM chat completions.
///
/// Each provider (Anthropic, OpenAI-compatible, etc.) implements this trait
/// to translate between provider-agnostic types and their wire format.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a message and return the complete response.
    async fn send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>;

    /// Send a message with deadline awareness.
    ///
    /// When the remaining time before `deadline` is less than
    /// `TYPICAL_CALL_DURATION + RETRY_BUFFER` (120s), abort the retry chain
    /// early instead of starting another attempt that would almost certainly
    /// be cancelled by the outer deadline anyway.
    ///
    /// Default implementation ignores the deadline and delegates to `send_message`.
    async fn send_message_with_deadline(
        &self,
        request: &LlmRequest,
        deadline: Option<Instant>,
    ) -> Result<LlmResponse, LlmError> {
        let _ = deadline;
        self.send_message(request).await
    }

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

    /// Conservative maximum output tokens for this provider.
    ///
    /// Used to warn users when `llm_max_tokens` exceeds the provider's typical limit.
    /// These are provider-level defaults -- individual models may differ.
    pub fn max_output_tokens(&self) -> u32 {
        match self {
            ProviderKind::Anthropic => 128_000,
            ProviderKind::OpenAi => 16_384,
            ProviderKind::OpenRouter => 128_000, // varies by model
            ProviderKind::Groq => 8_192,
            ProviderKind::Ollama => 131_072, // no hard limit
            ProviderKind::Mistral => 8_192,
            ProviderKind::Google => 65_536,
            ProviderKind::DeepSeek => 8_192,
            ProviderKind::MiniMax => 16_384,
            ProviderKind::Kimi => 8_192,
            ProviderKind::Qwen => 8_192,
        }
    }

    /// Whether this provider's model identifiers contain slashes (e.g., `qwen/qwen-plus`).
    /// Used to avoid misinterpreting model names as cross-provider switches.
    pub fn model_names_contain_slash(&self) -> bool {
        matches!(self, ProviderKind::OpenRouter)
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
        assert_eq!(ProviderKind::ALL.len(), 11);
        assert_eq!(ProviderKind::ALL[0], ProviderKind::Anthropic);
        assert_eq!(ProviderKind::ALL[10], ProviderKind::Qwen);
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

    // -- strip_internal_tags tests --

    #[test]
    fn test_strip_context_tag() {
        let input = r#"Hello <context type="tool_history" trust="metadata">
send_message({"text":"test"})
[TOOL_RESULT] → Message sent.
</context> world"#;
        assert_eq!(strip_internal_tags(input), "Hello  world");
    }

    #[test]
    fn test_strip_callback_result_tag() {
        let input = r#"Result: <callback_result trust="untrusted">task completed successfully</callback_result> done."#;
        assert_eq!(strip_internal_tags(input), "Result:  done.");
    }

    #[test]
    fn test_strip_task_health_with_nested_tags() {
        let input = "<task-health>\n<active-work-items>\n- item 1\n</active-work-items>\n<anomalies>\n- stuck task\n</anomalies>\n<task-health-instructions>\nCheck tasks.\n</task-health-instructions>\n</task-health>";
        assert_eq!(strip_internal_tags(input), "");
    }

    #[test]
    fn test_strip_rewind_reversals() {
        let input =
            r#"Done. <rewind_reversals trust="internal">reversed 3 facts</rewind_reversals>"#;
        assert_eq!(strip_internal_tags(input), "Done.");
    }

    #[test]
    fn test_strip_multiple_tags_preserves_text_between() {
        let input =
            "Before <context type=\"a\">x</context> middle <context type=\"b\">y</context> after";
        assert_eq!(strip_internal_tags(input), "Before  middle  after");
    }

    #[test]
    fn test_strip_no_tags_passthrough() {
        let input = "Just a normal response with no internal tags.";
        assert_eq!(strip_internal_tags(input), input);
    }

    #[test]
    fn test_strip_empty_after_strip() {
        let input = "<context type=\"tool_history\" trust=\"metadata\">only metadata</context>";
        assert_eq!(strip_internal_tags(input), "");
    }

    #[test]
    fn test_strip_partial_unclosed_tag_left_alone() {
        let input = "Hello <context type=\"tool_history\"> unclosed tag with no end";
        assert_eq!(strip_internal_tags(input), input);
    }

    #[test]
    fn test_strip_collapses_blank_lines() {
        let input = "Before\n\n<context type=\"x\">content</context>\n\nAfter";
        let result = strip_internal_tags(input);
        assert_eq!(result, "Before\n\nAfter");
    }

    #[test]
    fn test_strip_context_summary_tag() {
        let input =
            "Hello <context type=\"summary\" trust=\"data\">long summary here</context> bye";
        assert_eq!(strip_internal_tags(input), "Hello  bye");
    }

    #[test]
    fn test_strip_context_skill_tag() {
        let input =
            "<context type=\"skill\" trust=\"local\">skill prompt</context>\nActual response.";
        assert_eq!(strip_internal_tags(input), "Actual response.");
    }

    #[test]
    fn test_strip_standalone_nested_tag() {
        // LLM echoes only a nested tag without the outer <task-health>
        let input = "Status: <active-work-items>fix auth flow</active-work-items> checked.";
        assert_eq!(strip_internal_tags(input), "Status:  checked.");
    }

    #[test]
    fn test_strip_malformed_closing_bare_tag() {
        // #453: non-Anthropic models echo `context>` instead of `</context>`
        let input = r#"Hello.
<context type="tool_history" trust="metadata">
list_tasks({"source":"self_dev"}) → No tasks found...
context>
Bye."#;
        assert_eq!(strip_internal_tags(input), "Hello.\n\nBye.");
    }

    #[test]
    fn test_strip_malformed_closing_space_in_slash() {
        // LLM tokenization artifact: </ context> with space before tag name
        let input = r#"Result: <context type="summary">data here</ context> done."#;
        assert_eq!(strip_internal_tags(input), "Result:  done.");
    }

    #[test]
    fn test_strip_malformed_closing_space_after_angle() {
        // LLM tokenization artifact: < /context> with space after <
        let input = r#"X <callback_result trust="untrusted">result< /callback_result> Y"#;
        assert_eq!(strip_internal_tags(input), "X  Y");
    }

    #[test]
    fn test_strip_nested_with_malformed_outer_closing() {
        // Outer <task-health> has malformed closing, inner tags are well-formed
        let input =
            "<task-health>\n<active-work-items>\n- item\n</active-work-items>\ntask-health>";
        assert_eq!(strip_internal_tags(input), "");
    }

    #[test]
    fn test_strip_malformed_closing_bare_tag_with_trailing_space() {
        // Bare closing tag with trailing space before >
        let input =
            r#"Hi <context type="x">data</context> and <context type="y">more data context > end"#;
        assert_eq!(strip_internal_tags(input), "Hi  and  end");
    }

    // -- send_message_with_deadline default implementation tests --

    #[tokio::test]
    async fn test_send_message_with_deadline_none_delegates_to_send_message() {
        use super::mock::*;

        let provider = MockLlmProvider::builder()
            .response(text_response("Hello!"))
            .build();

        let request = LlmRequest {
            model: "test".into(),
            system: None,
            messages: vec![],
            tools: None,
            max_tokens: 100,
            thinking: None,
        };

        // None deadline should delegate to send_message (default trait impl)
        let result = provider
            .send_message_with_deadline(&request, None)
            .await
            .unwrap();
        assert_eq!(result.text(), "Hello!");
        assert_eq!(provider.calls_made(), 1);
    }

    #[tokio::test]
    async fn test_send_message_with_deadline_some_delegates_to_send_message() {
        use super::mock::*;

        let provider = MockLlmProvider::builder()
            .response(text_response("World!"))
            .build();

        let request = LlmRequest {
            model: "test".into(),
            system: None,
            messages: vec![],
            tools: None,
            max_tokens: 100,
            thinking: None,
        };

        // Some deadline (far future) should also delegate successfully
        let deadline = Instant::now() + std::time::Duration::from_secs(600);
        let result = provider
            .send_message_with_deadline(&request, Some(deadline))
            .await
            .unwrap();
        assert_eq!(result.text(), "World!");
        assert_eq!(provider.calls_made(), 1);
    }
}
