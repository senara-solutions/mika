use anyhow::{Context, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{Instrument, info, info_span, warn};

use crate::oauth::OAuthTokenManager;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const MAX_RETRIES: u32 = 3;
const OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat";

// -- Auth --

/// Check whether a credential string is an OAuth subscription token.
pub fn is_oauth_token(token: &str) -> bool {
    token.starts_with(OAUTH_TOKEN_PREFIX)
}

/// Anthropic authentication method, auto-detected from token prefix.
///
/// Three variants:
/// - `ApiKey` — standard `sk-ant-api*` keys, sent via `x-api-key` header
/// - `OAuthBearer` — raw static OAuth token (legacy/testing), sent via `Authorization: Bearer`
/// - `OAuthManaged` — managed token lifecycle with auto-refresh via `OAuthTokenManager`
#[derive(Clone)]
enum AnthropicAuth {
    ApiKey(String),
    OAuthBearer(String),
    OAuthManaged(Arc<OAuthTokenManager>),
}

impl std::fmt::Debug for AnthropicAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey(_) => write!(f, "AnthropicAuth::ApiKey([REDACTED])"),
            Self::OAuthBearer(_) => write!(f, "AnthropicAuth::OAuthBearer([REDACTED])"),
            Self::OAuthManaged(mgr) => write!(f, "AnthropicAuth::OAuthManaged({mgr:?})"),
        }
    }
}

impl AnthropicAuth {
    /// Auto-detect auth method from token prefix.
    /// Non-OAuth tokens use `ApiKey`; OAuth tokens use `OAuthBearer` (static).
    /// For managed OAuth with auto-refresh, use `from_oauth_token()` instead.
    fn from_token(token: String) -> Self {
        if token.starts_with(OAUTH_TOKEN_PREFIX) {
            Self::OAuthBearer(token)
        } else {
            Self::ApiKey(token)
        }
    }

    /// Create a managed OAuth auth from a subscription token.
    /// The manager handles token exchange and auto-refresh.
    fn from_oauth_token(subscription_token: String, home_dir: std::path::PathBuf) -> Self {
        let manager = crate::oauth::create_token_manager(&subscription_token, home_dir);
        Self::OAuthManaged(manager)
    }

    /// Whether this is an OAuth bearer token (static or managed).
    fn is_oauth(&self) -> bool {
        matches!(self, Self::OAuthBearer(_) | Self::OAuthManaged(_))
    }

    /// Whether this is a managed OAuth token with auto-refresh.
    fn is_oauth_managed(&self) -> bool {
        matches!(self, Self::OAuthManaged(_))
    }
}

// -- Request types --

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ThinkingConfig {
    #[serde(rename = "enabled")]
    Enabled { budget_tokens: u32 },
}

#[derive(Debug, Clone, Serialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: ToolResultBody,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    #[serde(rename = "image")]
    Image { source: ImageSource },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

/// Content blocks that can appear inside a `tool_result` content array.
///
/// The Claude API allows `tool_result.content` to be either a plain string
/// (shorthand for a single text block) or an array of text + image blocks.
/// This enum represents the array element types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolResultBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
}

/// The body of a `tool_result` content field.
///
/// Serializes as a plain string when text-only (backward compatible shorthand),
/// or as an array of `ToolResultBlock` when images are present.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultBody {
    Text(String),
    Blocks(Vec<ToolResultBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// -- Response types --

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesResponse {
    pub id: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u32>,
}

// -- Error response --

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
}

// -- Typed errors --

#[derive(Error, Debug)]
pub enum ClaudeApiError {
    #[error("Claude API HTTP error ({status}): {message}")]
    HttpError { status: u16, message: String },
    #[error("Claude API request failed")]
    Transport(#[from] reqwest::Error),
    #[error("Claude API response parse error")]
    ParseError(#[source] reqwest::Error),
}

// -- Client --

impl MessagesResponse {
    /// Extract the text content from the response, joining all text blocks.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Extract thinking text from the response, joining all thinking blocks.
    pub fn thinking(&self) -> Option<String> {
        let thinking: Vec<&str> = self
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
                _ => None,
            })
            .collect();
        if thinking.is_empty() {
            None
        } else {
            Some(thinking.join("\n\n"))
        }
    }

    /// Extract tool use blocks from the response.
    pub fn tool_calls(&self) -> Vec<&ContentBlock> {
        self.content
            .iter()
            .filter(|block| matches!(block, ContentBlock::ToolUse { .. }))
            .collect()
    }
}

#[derive(Clone)]
pub struct ClaudeClient {
    client: reqwest::Client,
    auth: AnthropicAuth,
    pub model: String,
    pub max_tokens: u32,
}

impl ClaudeClient {
    pub fn new(api_key: Option<String>, model: String, max_tokens: u32) -> Result<Self> {
        let credential = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "MIKA_LLM_API_KEY is required but not set. \
                     Set it to your LLM provider's API key \
                     (e.g., sk-ant-api03-... for Anthropic, sk-ant-oat01-... for OAuth, \
                     or your provider's key)."
                )
            })?;

        let auth = if is_oauth_token(&credential) {
            let home_dir = crate::home::resolve_home_dir()?;
            AnthropicAuth::from_oauth_token(credential, home_dir)
        } else {
            AnthropicAuth::from_token(credential)
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client");

        Ok(Self {
            client,
            auth,
            model,
            max_tokens,
        })
    }

    /// Create a placeholder client that cannot make API calls.
    /// Used by team mode TUI where the team engine creates its own clients.
    pub fn dummy() -> Self {
        Self {
            client: reqwest::Client::new(),
            auth: AnthropicAuth::ApiKey(String::new()),
            model: String::new(),
            max_tokens: 0,
        }
    }

    /// Send a message to Claude with retry on transient errors (429, 500, 529).
    pub async fn send_message(&self, request: &MessagesRequest) -> Result<MessagesResponse> {
        let span = info_span!(
            target: "mika::otel",
            "llm_call",
            model = %request.model,
            max_tokens = request.max_tokens,
        );

        // Set gen_ai semantic convention attributes for Langfuse generation classification
        #[cfg(feature = "telemetry")]
        {
            use tracing_opentelemetry::OpenTelemetrySpanExt;
            span.set_attribute("gen_ai.operation.name", "chat");
            span.set_attribute("gen_ai.provider.name", "anthropic");
            span.set_attribute("gen_ai.request.model", request.model.clone());
            span.set_attribute("gen_ai.request.max_tokens", request.max_tokens as i64);
        }

        let response = self
            .send_message_inner(request)
            .instrument(span.clone())
            .await?;

        // Set gen_ai response attributes after successful API call
        #[cfg(feature = "telemetry")]
        {
            use tracing_opentelemetry::OpenTelemetrySpanExt;
            span.set_attribute(
                "gen_ai.usage.input_tokens",
                response.usage.input_tokens as i64,
            );
            span.set_attribute(
                "gen_ai.usage.output_tokens",
                response.usage.output_tokens as i64,
            );
            span.set_attribute(
                "gen_ai.response.finish_reasons",
                format!("{:?}", response.stop_reason),
            );
        }

        Ok(response)
    }

    /// Inner implementation of send_message with retry logic.
    async fn send_message_inner(&self, request: &MessagesRequest) -> Result<MessagesResponse> {
        info!(
            model = %request.model,
            max_tokens = request.max_tokens,
            "llm_call started"
        );

        // Build the final auth header value upfront (non-retryable configuration error).
        // OAuth needs "Bearer <token>"; API key is the raw value.
        // Use an opaque error to avoid leaking the actual key/token value.
        let auth_header = match &self.auth {
            AnthropicAuth::ApiKey(k) => {
                HeaderValue::from_str(k).context("invalid API key characters")?
            }
            AnthropicAuth::OAuthBearer(t) => HeaderValue::from_str(&format!("Bearer {t}"))
                .context("invalid OAuth token characters")?,
            AnthropicAuth::OAuthManaged(manager) => {
                let token = manager.get_valid_token().await.context(
                    "OAuth token resolution failed. Run `mika setup --mode oauth` to authorize.",
                )?;
                HeaderValue::from_str(&format!("Bearer {token}"))
                    .context("invalid OAuth access token characters")?
            }
        };

        let mut last_error = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                warn!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    "retrying Claude API call"
                );
                tokio::time::sleep(delay).await;
            }

            match self.send_once(request, auth_header.clone()).await {
                Ok(response) => {
                    info!(
                        model = %request.model,
                        input_tokens = response.usage.input_tokens,
                        output_tokens = response.usage.output_tokens,
                        stop_reason = ?response.stop_reason,
                        "llm_call completed"
                    );
                    return Ok(response);
                }
                Err(e) => {
                    if attempt < MAX_RETRIES && is_retryable(&e) {
                        warn!(attempt, error = %e, "transient Claude API error");
                        last_error = Some(e);
                        continue;
                    }
                    // For managed OAuth, attempt a force-refresh on 401 before giving up.
                    if let ClaudeApiError::HttpError { status: 401, .. } = &e
                        && let AnthropicAuth::OAuthManaged(manager) = &self.auth
                        && let Ok(new_token) = manager.force_refresh().await
                        && let Ok(new_header) =
                            HeaderValue::from_str(&format!("Bearer {new_token}"))
                        && let Ok(response) = self.send_once(request, new_header).await
                    {
                        info!(
                            model = %request.model,
                            "llm_call succeeded after OAuth force-refresh"
                        );
                        return Ok(response);
                    }

                    return Err(match &e {
                        ClaudeApiError::HttpError { status: 401, .. } => {
                            let hint = if self.auth.is_oauth_managed() {
                                "Authentication failed after token refresh. \
                                 Run `mika setup --mode oauth` to re-authorize."
                            } else if self.auth.is_oauth() {
                                "Authentication failed. Your OAuth token may have expired. \
                                 Run `mika setup --mode oauth` to get a new one."
                            } else {
                                "Authentication failed. Check that MIKA_LLM_API_KEY is set to a valid Anthropic API key."
                            };
                            anyhow::Error::from(e).context(hint)
                        }
                        ClaudeApiError::HttpError { status: 429, .. } => anyhow::Error::from(e)
                            .context("Claude API is busy. Please wait a moment and try again."),
                        ClaudeApiError::HttpError { status, .. } if *status >= 500 => {
                            anyhow::Error::from(e).context(
                                "Claude API is temporarily unavailable. Please try again shortly.",
                            )
                        }
                        ClaudeApiError::Transport(_) => anyhow::Error::from(e).context(
                            "Could not connect to Claude API. Check your internet connection.",
                        ),
                        ClaudeApiError::ParseError(_) => anyhow::Error::from(e)
                            .context("Received an unexpected response from Claude API."),
                        ClaudeApiError::HttpError { .. } => anyhow::Error::from(e)
                            .context("Claude API returned an unexpected error. Please try again."),
                    });
                }
            }
        }

        Err(last_error
            .map(|e| match &e {
                ClaudeApiError::HttpError { status, .. } if *status >= 500 => {
                    anyhow::Error::from(e)
                        .context("Claude API is temporarily unavailable. Please try again shortly.")
                }
                _ => anyhow::Error::from(e)
                    .context("Claude API is busy. Please wait a moment and try again."),
            })
            .unwrap_or_else(|| anyhow::anyhow!("max retries exceeded")))
    }

    async fn send_once(
        &self,
        request: &MessagesRequest,
        auth_header: HeaderValue,
    ) -> std::result::Result<MessagesResponse, ClaudeApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("anthropic-version", HeaderValue::from_static(API_VERSION));

        // Auth header — pre-built in send_message_inner() with correct format for each method
        match &self.auth {
            AnthropicAuth::ApiKey(_) => {
                headers.insert("x-api-key", auth_header);
            }
            AnthropicAuth::OAuthBearer(_) | AnthropicAuth::OAuthManaged(_) => {
                headers.insert(AUTHORIZATION, auth_header);
            }
        }

        // Beta headers — collect all needed betas, then set once (avoids insert-replace bug)
        let mut betas: Vec<&str> = Vec::new();
        if self.auth.is_oauth() {
            betas.push("oauth-2025-04-20");
        }
        if request.thinking.is_some() {
            betas.push("interleaved-thinking-2025-05-14");
        }
        if !betas.is_empty() {
            let beta_value = betas.join(",");
            headers.insert(
                "anthropic-beta",
                HeaderValue::from_str(&beta_value).expect("static beta values are valid"),
            );
        }

        let response = self
            .client
            .post(API_URL)
            .headers(headers)
            .json(request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            // Log the body at warn level but do NOT include it in the error
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiErrorResponse>(&body)
                .map(|e| e.error.message)
                .unwrap_or_else(|_| {
                    // Truncate raw body to avoid leaking proxy/CDN internals
                    let truncated: String = body.chars().take(200).collect();
                    format!("unexpected error response (HTTP {status_code}): {truncated}")
                });
            warn!(status = status_code, error_message = %message, "Claude API error response");
            return Err(ClaudeApiError::HttpError {
                status: status_code,
                message,
            });
        }

        let response: MessagesResponse =
            response.json().await.map_err(ClaudeApiError::ParseError)?;

        Ok(response)
    }
}

fn is_retryable(error: &ClaudeApiError) -> bool {
    match error {
        ClaudeApiError::HttpError { status, .. } => matches!(status, 429 | 500 | 529),
        ClaudeApiError::Transport(e) => e.is_timeout(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_response() {
        let json = r#"{
            "id": "msg_01",
            "content": [{"type": "text", "text": "Hello!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.text(), "Hello!");
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert_eq!(resp.usage.input_tokens, 10);
    }

    #[test]
    fn test_deserialize_tool_use_response() {
        let json = r#"{
            "id": "msg_02",
            "content": [
                {"type": "text", "text": "Let me update that."},
                {"type": "tool_use", "id": "tu_1", "name": "update_core_memory", "input": {"key": "user_summary", "value": "Likes coffee"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 50, "output_tokens": 30}
        }"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        assert_eq!(resp.tool_calls().len(), 1);
        assert_eq!(resp.text(), "Let me update that.");
    }

    #[test]
    fn test_serialize_request() {
        let req = MessagesRequest {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 4096,
            system: Some("You are Mika.".into()),
            messages: vec![Message {
                role: "user".into(),
                content: MessageContent::Text("Hello".into()),
            }],
            tools: None,
            thinking: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("claude-sonnet"));
        assert!(json.contains("Hello"));
    }

    #[test]
    fn test_new_trims_api_key_whitespace() {
        let client =
            ClaudeClient::new(Some("  sk-ant-test  ".into()), "model".into(), 100).unwrap();
        assert!(matches!(client.auth, AnthropicAuth::ApiKey(ref k) if k == "sk-ant-test"));
    }

    #[test]
    fn test_new_rejects_whitespace_only_key() {
        let result = ClaudeClient::new(Some("   ".into()), "model".into(), 100);
        let Err(e) = result else {
            panic!("should reject whitespace-only key");
        };
        assert!(e.to_string().contains("required but not set"));
    }

    #[test]
    fn test_new_rejects_none_key() {
        let result = ClaudeClient::new(None, "model".into(), 100);
        let Err(e) = result else {
            panic!("should reject None key");
        };
        assert!(e.to_string().contains("required but not set"));
    }

    // -- AnthropicAuth tests --

    #[test]
    fn test_auth_auto_detect_api_key() {
        let auth = AnthropicAuth::from_token("sk-ant-api03-abc123".into());
        assert!(matches!(auth, AnthropicAuth::ApiKey(_)));
        assert!(!auth.is_oauth());
    }

    #[test]
    fn test_auth_auto_detect_oauth_token() {
        // from_token() creates OAuthBearer (static); from_oauth_token() creates OAuthManaged
        let auth = AnthropicAuth::from_token("sk-ant-oat01-abc123def456".into());
        assert!(matches!(auth, AnthropicAuth::OAuthBearer(_)));
        assert!(auth.is_oauth());
        assert!(!auth.is_oauth_managed());
    }

    #[test]
    fn test_auth_unknown_prefix_falls_back_to_api_key() {
        let auth = AnthropicAuth::from_token("some-random-key".into());
        assert!(matches!(auth, AnthropicAuth::ApiKey(_)));
        assert!(!auth.is_oauth());
    }

    #[test]
    fn test_new_with_oauth_token() {
        let client =
            ClaudeClient::new(Some("sk-ant-oat01-test-token".into()), "model".into(), 100).unwrap();
        assert!(client.auth.is_oauth());
        assert!(client.auth.is_oauth_managed());
    }

    #[test]
    fn test_new_with_api_key() {
        let client =
            ClaudeClient::new(Some("sk-ant-api03-test-key".into()), "model".into(), 100).unwrap();
        assert!(!client.auth.is_oauth());
        assert!(!client.auth.is_oauth_managed());
    }

    #[test]
    fn test_oauth_managed_variant() {
        let tmp = tempfile::tempdir().unwrap();
        let manager =
            crate::oauth::create_token_manager("sk-ant-oat01-test", tmp.path().to_path_buf());
        let auth = AnthropicAuth::OAuthManaged(manager);
        assert!(auth.is_oauth());
        assert!(auth.is_oauth_managed());
        // Debug should not leak the subscription token
        let debug = format!("{:?}", auth);
        assert!(!debug.contains("sk-ant-oat01-test"));
        assert!(debug.contains("OAuthManaged"));
    }

    #[test]
    fn test_beta_headers_combine_oauth_and_thinking() {
        let betas: Vec<&str> = vec!["oauth-2025-04-20", "interleaved-thinking-2025-05-14"];
        let combined = betas.join(",");
        assert_eq!(combined, "oauth-2025-04-20,interleaved-thinking-2025-05-14");
        assert!(HeaderValue::from_str(&combined).is_ok());
    }

    #[test]
    fn test_tool_definition_serialization() {
        let tool = ToolDefinition {
            name: "search_memory".into(),
            description: "Search the user's memory".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query"}
                },
                "required": ["query"]
            }),
        };
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["name"], "search_memory");
    }

    #[test]
    fn test_serialize_thinking_config() {
        let config = ThinkingConfig::Enabled {
            budget_tokens: 10_000,
        };
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["type"], "enabled");
        assert_eq!(json["budget_tokens"], 10_000);
    }

    #[test]
    fn test_serialize_request_with_thinking() {
        let req = MessagesRequest {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 16384,
            system: None,
            messages: vec![],
            tools: None,
            thinking: Some(ThinkingConfig::Enabled {
                budget_tokens: 10_000,
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"thinking\""));
        assert!(json.contains("\"budget_tokens\":10000"));
    }

    #[test]
    fn test_serialize_request_without_thinking_omits_field() {
        let req = MessagesRequest {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 4096,
            system: None,
            messages: vec![],
            tools: None,
            thinking: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("thinking"));
    }

    #[test]
    fn test_deserialize_thinking_block() {
        let json = r#"{
            "id": "msg_03",
            "content": [
                {"type": "thinking", "thinking": "Let me reason about this...", "signature": "sig123"},
                {"type": "text", "text": "The answer is 42."}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 100, "output_tokens": 50}
        }"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.text(), "The answer is 42.");
        assert_eq!(resp.thinking().unwrap(), "Let me reason about this...");
    }

    #[test]
    fn test_thinking_returns_none_when_no_thinking_blocks() {
        let json = r#"{
            "id": "msg_04",
            "content": [{"type": "text", "text": "Hello!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        assert!(resp.thinking().is_none());
    }

    #[test]
    fn test_deserialize_usage_with_cache_fields() {
        let json = r#"{
            "id": "msg_05",
            "content": [{"type": "text", "text": "Hi"}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 200,
                "cache_read_input_tokens": 80
            }
        }"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.usage.cache_creation_input_tokens, Some(200));
        assert_eq!(resp.usage.cache_read_input_tokens, Some(80));
    }

    #[test]
    fn test_deserialize_usage_without_cache_fields() {
        let json = r#"{
            "id": "msg_06",
            "content": [{"type": "text", "text": "Hi"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        }"#;
        let resp: MessagesResponse = serde_json::from_str(json).unwrap();
        assert!(resp.usage.cache_creation_input_tokens.is_none());
        assert!(resp.usage.cache_read_input_tokens.is_none());
    }

    #[test]
    fn test_serialize_image_content_block() {
        let block = ContentBlock::Image {
            source: ImageSource {
                source_type: "base64".into(),
                media_type: "image/png".into(),
                data: "iVBOR...".into(),
            },
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(json["source"]["type"], "base64");
        assert_eq!(json["source"]["media_type"], "image/png");
    }

    // -- ToolResultBody serde round-trip tests --

    #[test]
    fn test_tool_result_body_text_serializes_as_string() {
        let body = ToolResultBody::Text("hello".into());
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json, serde_json::json!("hello"));
    }

    #[test]
    fn test_tool_result_body_blocks_serializes_as_array() {
        let body = ToolResultBody::Blocks(vec![
            ToolResultBlock::Text {
                text: "Screenshot taken.".into(),
            },
            ToolResultBlock::Image {
                source: ImageSource {
                    source_type: "base64".into(),
                    media_type: "image/png".into(),
                    data: "iVBOR...".into(),
                },
            },
        ]);
        let json = serde_json::to_value(&body).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "Screenshot taken.");
        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["source"]["media_type"], "image/png");
    }

    #[test]
    fn test_tool_result_body_text_round_trip() {
        let body = ToolResultBody::Text("result text".into());
        let json = serde_json::to_string(&body).unwrap();
        let deserialized: ToolResultBody = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, ToolResultBody::Text(ref s) if s == "result text"));
    }

    #[test]
    fn test_tool_result_body_blocks_round_trip() {
        let body = ToolResultBody::Blocks(vec![
            ToolResultBlock::Text {
                text: "Done.".into(),
            },
            ToolResultBlock::Image {
                source: ImageSource {
                    source_type: "base64".into(),
                    media_type: "image/jpeg".into(),
                    data: "/9j/4AAQ...".into(),
                },
            },
        ]);
        let json = serde_json::to_string(&body).unwrap();
        let deserialized: ToolResultBody = serde_json::from_str(&json).unwrap();
        match deserialized {
            ToolResultBody::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                assert!(matches!(&blocks[0], ToolResultBlock::Text { text } if text == "Done."));
                assert!(
                    matches!(&blocks[1], ToolResultBlock::Image { source } if source.media_type == "image/jpeg")
                );
            }
            _ => panic!("expected Blocks variant"),
        }
    }

    #[test]
    fn test_tool_result_content_block_text_only() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tu_1".into(),
            content: ToolResultBody::Text("success".into()),
            is_error: None,
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["tool_use_id"], "tu_1");
        assert_eq!(json["content"], "success");
        assert!(json.get("is_error").is_none());
    }

    #[test]
    fn test_tool_result_content_block_with_images() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tu_2".into(),
            content: ToolResultBody::Blocks(vec![
                ToolResultBlock::Text {
                    text: "Screenshot captured.".into(),
                },
                ToolResultBlock::Image {
                    source: ImageSource {
                        source_type: "base64".into(),
                        media_type: "image/png".into(),
                        data: "abc123".into(),
                    },
                },
            ]),
            is_error: None,
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "tool_result");
        assert_eq!(json["tool_use_id"], "tu_2");
        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
    }
}
