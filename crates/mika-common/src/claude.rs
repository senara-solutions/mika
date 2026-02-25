use anyhow::{Context, Result};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, warn};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const MAX_RETRIES: u32 = 3;

// -- Request types --

#[derive(Debug, Clone, Serialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
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
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
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
    api_key: String,
    pub model: String,
    pub max_tokens: u32,
}

impl ClaudeClient {
    pub fn new(api_key: Option<String>, model: String, max_tokens: u32) -> Result<Self> {
        let api_key = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("MIKA_ANTHROPIC_API_KEY is required but not set"))?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client");

        Ok(Self {
            client,
            api_key,
            model,
            max_tokens,
        })
    }

    /// Send a message to Claude with retry on transient errors (429, 500, 529).
    pub async fn send_message(&self, request: &MessagesRequest) -> Result<MessagesResponse> {
        // Validate API key header value upfront (non-retryable configuration error).
        // Use an opaque message to avoid leaking the actual key value.
        let api_key_header =
            HeaderValue::from_str(&self.api_key).context("invalid API key characters")?;

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

            match self.send_once(request, api_key_header.clone()).await {
                Ok(response) => {
                    debug!(
                        input_tokens = response.usage.input_tokens,
                        output_tokens = response.usage.output_tokens,
                        stop_reason = ?response.stop_reason,
                        "Claude API response"
                    );
                    return Ok(response);
                }
                Err(e) => {
                    if attempt < MAX_RETRIES && is_retryable(&e) {
                        warn!(attempt, error = %e, "transient Claude API error");
                        last_error = Some(e);
                        continue;
                    }
                    return Err(match &e {
                        ClaudeApiError::HttpError { status: 401, .. } => {
                            anyhow::Error::from(e).context(
                                "Authentication failed. Check that MIKA_ANTHROPIC_API_KEY is set to a valid Anthropic API key.",
                            )
                        }
                        ClaudeApiError::HttpError { status: 429, .. } => {
                            anyhow::Error::from(e).context(
                                "Claude API is busy. Please wait a moment and try again.",
                            )
                        }
                        ClaudeApiError::HttpError { status, .. } if *status >= 500 => {
                            anyhow::Error::from(e).context(
                                "Claude API is temporarily unavailable. Please try again shortly.",
                            )
                        }
                        ClaudeApiError::Transport(_) => {
                            anyhow::Error::from(e).context(
                                "Could not connect to Claude API. Check your internet connection.",
                            )
                        }
                        ClaudeApiError::ParseError(_) => {
                            anyhow::Error::from(e).context(
                                "Received an unexpected response from Claude API.",
                            )
                        }
                        _ => e.into(),
                    });
                }
            }
        }

        Err(last_error
            .map(|e| {
                anyhow::Error::from(e)
                    .context("Claude API is busy. Please wait a moment and try again.")
            })
            .unwrap_or_else(|| anyhow::anyhow!("max retries exceeded")))
    }

    async fn send_once(
        &self,
        request: &MessagesRequest,
        api_key_header: HeaderValue,
    ) -> std::result::Result<MessagesResponse, ClaudeApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("x-api-key", api_key_header);
        headers.insert("anthropic-version", HeaderValue::from_static(API_VERSION));

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
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("claude-sonnet"));
        assert!(json.contains("Hello"));
    }

    #[test]
    fn test_new_trims_api_key_whitespace() {
        let client =
            ClaudeClient::new(Some("  sk-ant-test  ".into()), "model".into(), 100).unwrap();
        assert_eq!(client.api_key, "sk-ant-test");
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
}
