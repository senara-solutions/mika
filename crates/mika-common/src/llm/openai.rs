use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use super::error::LlmError;
use super::types::*;
use super::{LlmProvider, ProviderKind};

// -- OpenAI wire types --

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    max_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OpenAiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

/// OpenAI content can be a plain string or an array of content parts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum OpenAiContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum OpenAiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OpenAiImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiImageUrl {
    url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunctionDef,
}

#[derive(Serialize)]
struct OpenAiFunctionDef {
    name: String,
    description: String,
    parameters: Value,
}

// -- Response types --

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

// -- Error response --

#[derive(Deserialize)]
struct OpenAiErrorResponse {
    error: OpenAiErrorDetail,
}

#[derive(Deserialize)]
struct OpenAiErrorDetail {
    message: String,
}

// -- Provider implementation --

const MAX_RETRIES: u32 = 3;

/// OpenAI-compatible provider that works with OpenAI, Ollama, vLLM, Groq, etc.
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    max_tokens: u32,
    provider_kind: ProviderKind,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        base_url: String,
        api_key: Option<String>,
        model: String,
        max_tokens: u32,
        provider_kind: ProviderKind,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client");

        // Normalize base_url: strip trailing slash
        let base_url = base_url.trim_end_matches('/').to_string();

        Self {
            client,
            base_url,
            api_key,
            model,
            max_tokens,
            provider_kind,
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn models_url(&self) -> String {
        format!("{}/models", self.base_url)
    }

    async fn send_once(&self, request: &OpenAiRequest) -> Result<OpenAiResponse, LlmError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(ref key) = self.api_key {
            let auth = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|e| LlmError::ProviderError(format!("invalid API key: {e}")))?;
            headers.insert(AUTHORIZATION, auth);
        }

        let response = self
            .client
            .post(self.chat_url())
            .headers(headers)
            .json(request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<OpenAiErrorResponse>(&body)
                .map(|e| e.error.message)
                .unwrap_or_else(|_| {
                    let truncated: String = body.chars().take(200).collect();
                    format!("HTTP {status_code}: {truncated}")
                });
            warn!(status = status_code, error_message = %message, "OpenAI-compatible API error");
            let retryable = matches!(status_code, 429 | 500 | 503);
            return Err(LlmError::HttpError {
                status: status_code,
                message,
                retryable,
            });
        }

        let resp: OpenAiResponse = response
            .json()
            .await
            .map_err(|e| LlmError::ParseError(format!("failed to parse response: {e}")))?;

        Ok(resp)
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let openai_request = to_openai_request(request);

        info!(
            model = %request.model,
            max_tokens = request.max_tokens,
            provider = %self.provider_kind,
            "llm_call started"
        );

        let mut last_error = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                warn!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    "retrying OpenAI-compatible API call"
                );
                tokio::time::sleep(delay).await;
            }

            match self.send_once(&openai_request).await {
                Ok(response) => {
                    let llm_response = from_openai_response(response)?;
                    info!(
                        model = %request.model,
                        input_tokens = llm_response.usage.input_tokens,
                        output_tokens = llm_response.usage.output_tokens,
                        stop_reason = ?llm_response.stop_reason,
                        provider = %self.provider_kind,
                        "llm_call completed"
                    );
                    return Ok(llm_response);
                }
                Err(e) => {
                    if attempt < MAX_RETRIES && e.is_retryable() {
                        warn!(attempt, error = %e, "transient API error");
                        last_error = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::ProviderError("max retries exceeded".into())))
    }

    fn provider_name(&self) -> &str {
        match self.provider_kind {
            ProviderKind::OpenAi => "openai",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Groq => "groq",
            _ => "openai-compatible",
        }
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    fn supports_tool_calling(&self) -> bool {
        true
    }

    fn supports_vision(&self) -> bool {
        // Most OpenAI-compatible providers support vision for their multimodal models.
        // Conservative default; can be made configurable later.
        matches!(self.provider_kind, ProviderKind::OpenAi)
    }

    fn supports_extended_thinking(&self) -> bool {
        false
    }

    async fn check_health(&self) -> Result<(), LlmError> {
        let mut headers = HeaderMap::new();
        if let Some(ref key) = self.api_key {
            let auth = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|e| LlmError::ProviderError(format!("invalid API key: {e}")))?;
            headers.insert(AUTHORIZATION, auth);
        }

        let response = self
            .client
            .get(self.models_url())
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(LlmError::HttpError {
                status: response.status().as_u16(),
                message: "health check failed".into(),
                retryable: false,
            });
        }

        Ok(())
    }
}

// -- Translation: LlmRequest → OpenAiRequest --

fn to_openai_request(req: &LlmRequest) -> OpenAiRequest {
    let mut messages = Vec::new();

    // System prompt goes as a system role message
    if let Some(ref system) = req.system {
        messages.push(OpenAiMessage {
            role: "system".into(),
            content: Some(OpenAiContent::Text(system.clone())),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    // Convert conversation messages
    for msg in &req.messages {
        messages.extend(to_openai_messages(msg));
    }

    let tools = req.tools.as_ref().map(|tools| {
        tools
            .iter()
            .map(|t| OpenAiTool {
                tool_type: "function".into(),
                function: OpenAiFunctionDef {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect()
    });

    OpenAiRequest {
        model: req.model.clone(),
        messages,
        tools,
        max_tokens: req.max_tokens,
    }
}

/// Convert a single `LlmMessage` into one or more `OpenAiMessage`s.
///
/// Most messages map 1:1, but tool result messages need special handling:
/// each `ToolResult` block becomes a separate `role: "tool"` message.
fn to_openai_messages(msg: &LlmMessage) -> Vec<OpenAiMessage> {
    match msg.role {
        LlmRole::User => {
            vec![OpenAiMessage {
                role: "user".into(),
                content: Some(to_openai_content(&msg.content)),
                tool_calls: None,
                tool_call_id: None,
            }]
        }
        LlmRole::Assistant => {
            // Check if content has tool calls — they go in the `tool_calls` field
            match &msg.content {
                LlmContent::Blocks(blocks) => {
                    let mut tool_calls = Vec::new();
                    let mut text_parts = Vec::new();

                    for block in blocks {
                        match block {
                            LlmContentBlock::ToolCall {
                                id,
                                name,
                                arguments,
                            } => {
                                tool_calls.push(OpenAiToolCall {
                                    id: id.clone(),
                                    call_type: "function".into(),
                                    function: OpenAiFunction {
                                        name: name.clone(),
                                        arguments: serde_json::to_string(arguments)
                                            .unwrap_or_default(),
                                    },
                                });
                            }
                            LlmContentBlock::Text(t) => {
                                text_parts.push(t.clone());
                            }
                            _ => {}
                        }
                    }

                    let content = if text_parts.is_empty() {
                        None
                    } else {
                        Some(OpenAiContent::Text(text_parts.join("")))
                    };

                    let tool_calls = if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    };

                    vec![OpenAiMessage {
                        role: "assistant".into(),
                        content,
                        tool_calls,
                        tool_call_id: None,
                    }]
                }
                LlmContent::Text(t) => {
                    vec![OpenAiMessage {
                        role: "assistant".into(),
                        content: Some(OpenAiContent::Text(t.clone())),
                        tool_calls: None,
                        tool_call_id: None,
                    }]
                }
            }
        }
        LlmRole::Tool => {
            // Each tool result becomes a separate message with role="tool"
            match &msg.content {
                LlmContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        LlmContentBlock::ToolResult {
                            tool_call_id,
                            content,
                            ..
                        } => {
                            let text = match content {
                                LlmToolResultContent::Text(t) => t.clone(),
                                LlmToolResultContent::Blocks(parts) => parts
                                    .iter()
                                    .filter_map(|p| match p {
                                        LlmToolResultBlock::Text(t) => Some(t.as_str()),
                                        LlmToolResultBlock::Image(_) => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                            };
                            Some(OpenAiMessage {
                                role: "tool".into(),
                                content: Some(OpenAiContent::Text(text)),
                                tool_calls: None,
                                tool_call_id: Some(tool_call_id.clone()),
                            })
                        }
                        _ => None,
                    })
                    .collect(),
                LlmContent::Text(t) => {
                    vec![OpenAiMessage {
                        role: "tool".into(),
                        content: Some(OpenAiContent::Text(t.clone())),
                        tool_calls: None,
                        tool_call_id: None,
                    }]
                }
            }
        }
    }
}

fn to_openai_content(content: &LlmContent) -> OpenAiContent {
    match content {
        LlmContent::Text(t) => OpenAiContent::Text(t.clone()),
        LlmContent::Blocks(blocks) => {
            let parts: Vec<OpenAiContentPart> = blocks
                .iter()
                .filter_map(|b| match b {
                    LlmContentBlock::Text(t) => Some(OpenAiContentPart::Text { text: t.clone() }),
                    LlmContentBlock::Image(img) => {
                        let data_uri = format!("data:{};base64,{}", img.media_type, img.data);
                        Some(OpenAiContentPart::ImageUrl {
                            image_url: OpenAiImageUrl { url: data_uri },
                        })
                    }
                    // ToolCall/ToolResult blocks are handled separately
                    _ => None,
                })
                .collect();

            if parts.len() == 1
                && let OpenAiContentPart::Text { ref text } = parts[0]
            {
                return OpenAiContent::Text(text.clone());
            }
            OpenAiContent::Parts(parts)
        }
    }
}

// -- Translation: OpenAiResponse → LlmResponse --

fn from_openai_response(resp: OpenAiResponse) -> Result<LlmResponse, LlmError> {
    let choice = resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| LlmError::ParseError("no choices in response".into()))?;

    let mut content = Vec::new();

    // Extract text content
    if let Some(text_content) = choice.message.content {
        match text_content {
            OpenAiContent::Text(t) if !t.is_empty() => {
                content.push(LlmResponseContent::Text(t));
            }
            OpenAiContent::Parts(parts) => {
                for part in parts {
                    if let OpenAiContentPart::Text { text } = part
                        && !text.is_empty()
                    {
                        content.push(LlmResponseContent::Text(text));
                    }
                }
            }
            _ => {}
        }
    }

    // Extract tool calls
    if let Some(tool_calls) = choice.message.tool_calls {
        for tc in tool_calls {
            let arguments =
                serde_json::from_str::<Value>(&tc.function.arguments).unwrap_or_else(|e| {
                    warn!(
                        tool = %tc.function.name,
                        error = %e,
                        raw = %tc.function.arguments,
                        "failed to parse tool call arguments as JSON, wrapping as string"
                    );
                    Value::String(tc.function.arguments.clone())
                });
            content.push(LlmResponseContent::ToolCall {
                id: tc.id,
                name: tc.function.name,
                arguments,
            });
        }
    }

    let stop_reason = match choice.finish_reason.as_deref() {
        Some("tool_calls") => LlmStopReason::ToolUse,
        Some("length") => LlmStopReason::MaxTokens,
        Some("content_filter") => LlmStopReason::ContentFilter,
        _ => LlmStopReason::EndTurn,
    };

    let usage = resp.usage.map_or_else(LlmUsage::default, |u| LlmUsage {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    });

    Ok(LlmResponse {
        content,
        reasoning: None,
        stop_reason,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_to_openai_request_basic() {
        let req = LlmRequest {
            model: "gpt-4o".into(),
            system: Some("You are helpful.".into()),
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Text("Hello".into()),
            }],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let openai = to_openai_request(&req);
        assert_eq!(openai.model, "gpt-4o");
        assert_eq!(openai.messages.len(), 2); // system + user
        assert_eq!(openai.messages[0].role, "system");
        assert_eq!(openai.messages[1].role, "user");
    }

    #[test]
    fn test_to_openai_request_thinking_ignored() {
        use crate::claude::ThinkingConfig;

        let req = LlmRequest {
            model: "gpt-4o".into(),
            system: None,
            messages: vec![],
            tools: None,
            max_tokens: 4096,
            thinking: Some(ThinkingConfig::Enabled {
                budget_tokens: 10_000,
            }),
        };

        let openai = to_openai_request(&req);
        // Thinking is silently ignored — not present in OpenAI request
        let json = serde_json::to_value(&openai).unwrap();
        assert!(!json.to_string().contains("thinking"));
    }

    #[test]
    fn test_to_openai_request_with_tools() {
        let req = LlmRequest {
            model: "gpt-4o".into(),
            system: None,
            messages: vec![],
            tools: Some(vec![LlmToolDefinition {
                name: "search".into(),
                description: "Search memory".into(),
                parameters: json!({ "type": "object" }),
            }]),
            max_tokens: 4096,
            thinking: None,
        };

        let openai = to_openai_request(&req);
        let tools = openai.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_type, "function");
        assert_eq!(tools[0].function.name, "search");
        assert_eq!(tools[0].function.parameters, json!({ "type": "object" }));
    }

    #[test]
    fn test_to_openai_assistant_with_tool_calls() {
        let msg = LlmMessage {
            role: LlmRole::Assistant,
            content: LlmContent::Blocks(vec![
                LlmContentBlock::Text("Let me search.".into()),
                LlmContentBlock::ToolCall {
                    id: "call_1".into(),
                    name: "search".into(),
                    arguments: json!({"query": "test"}),
                },
            ]),
        };

        let openai_msgs = to_openai_messages(&msg);
        assert_eq!(openai_msgs.len(), 1);
        assert_eq!(openai_msgs[0].role, "assistant");
        // Text goes in content
        assert!(matches!(
            &openai_msgs[0].content,
            Some(OpenAiContent::Text(t)) if t == "Let me search."
        ));
        // Tool calls go in tool_calls field
        let tool_calls = openai_msgs[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
        assert_eq!(tool_calls[0].function.name, "search");
        // Arguments are serialized as JSON string
        assert_eq!(tool_calls[0].function.arguments, r#"{"query":"test"}"#);
    }

    #[test]
    fn test_to_openai_tool_result_messages() {
        let msg = LlmMessage {
            role: LlmRole::Tool,
            content: LlmContent::Blocks(vec![
                LlmContentBlock::ToolResult {
                    tool_call_id: "call_1".into(),
                    content: LlmToolResultContent::Text("result 1".into()),
                    is_error: false,
                },
                LlmContentBlock::ToolResult {
                    tool_call_id: "call_2".into(),
                    content: LlmToolResultContent::Text("result 2".into()),
                    is_error: true,
                },
            ]),
        };

        let openai_msgs = to_openai_messages(&msg);
        // Each tool result becomes a separate message
        assert_eq!(openai_msgs.len(), 2);
        assert_eq!(openai_msgs[0].role, "tool");
        assert_eq!(openai_msgs[0].tool_call_id, Some("call_1".into()));
        assert_eq!(openai_msgs[1].role, "tool");
        assert_eq!(openai_msgs[1].tool_call_id, Some("call_2".into()));
    }

    #[test]
    fn test_to_openai_image_content() {
        let msg = LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![
                LlmContentBlock::Image(LlmImage {
                    media_type: "image/png".into(),
                    data: "iVBOR...".into(),
                }),
                LlmContentBlock::Text("What is this?".into()),
            ]),
        };

        let openai_msgs = to_openai_messages(&msg);
        assert_eq!(openai_msgs.len(), 1);
        match &openai_msgs[0].content {
            Some(OpenAiContent::Parts(parts)) => {
                assert_eq!(parts.len(), 2);
                match &parts[0] {
                    OpenAiContentPart::ImageUrl { image_url } => {
                        assert!(image_url.url.starts_with("data:image/png;base64,"));
                    }
                    _ => panic!("expected ImageUrl"),
                }
            }
            _ => panic!("expected Parts content"),
        }
    }

    #[test]
    fn test_from_openai_response_text() {
        let resp = OpenAiResponse {
            choices: vec![OpenAiChoice {
                message: OpenAiMessage {
                    role: "assistant".into(),
                    content: Some(OpenAiContent::Text("Hello!".into())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: Some(OpenAiUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            }),
        };

        let llm = from_openai_response(resp).unwrap();
        assert_eq!(llm.text(), "Hello!");
        assert_eq!(llm.stop_reason, LlmStopReason::EndTurn);
        assert_eq!(llm.usage.input_tokens, 10);
        assert_eq!(llm.usage.output_tokens, 5);
        assert!(llm.reasoning.is_none());
    }

    #[test]
    fn test_from_openai_response_tool_calls() {
        let resp = OpenAiResponse {
            choices: vec![OpenAiChoice {
                message: OpenAiMessage {
                    role: "assistant".into(),
                    content: Some(OpenAiContent::Text("Searching...".into())),
                    tool_calls: Some(vec![OpenAiToolCall {
                        id: "call_abc".into(),
                        call_type: "function".into(),
                        function: OpenAiFunction {
                            name: "search".into(),
                            arguments: r#"{"query":"test"}"#.into(),
                        },
                    }]),
                    tool_call_id: None,
                },
                finish_reason: Some("tool_calls".into()),
            }],
            usage: None,
        };

        let llm = from_openai_response(resp).unwrap();
        assert_eq!(llm.stop_reason, LlmStopReason::ToolUse);
        assert!(llm.has_tool_calls());
        assert_eq!(llm.text(), "Searching...");

        match &llm.content[1] {
            LlmResponseContent::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_abc");
                assert_eq!(name, "search");
                assert_eq!(arguments, &json!({"query": "test"}));
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_from_openai_response_malformed_arguments() {
        let resp = OpenAiResponse {
            choices: vec![OpenAiChoice {
                message: OpenAiMessage {
                    role: "assistant".into(),
                    content: None,
                    tool_calls: Some(vec![OpenAiToolCall {
                        id: "call_1".into(),
                        call_type: "function".into(),
                        function: OpenAiFunction {
                            name: "search".into(),
                            arguments: "not valid json{".into(),
                        },
                    }]),
                    tool_call_id: None,
                },
                finish_reason: Some("tool_calls".into()),
            }],
            usage: None,
        };

        // Should not error — malformed arguments are wrapped as a string value
        let llm = from_openai_response(resp).unwrap();
        match &llm.content[0] {
            LlmResponseContent::ToolCall { arguments, .. } => {
                assert_eq!(arguments, &Value::String("not valid json{".into()));
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_from_openai_response_no_choices_fails() {
        let resp = OpenAiResponse {
            choices: vec![],
            usage: None,
        };
        assert!(from_openai_response(resp).is_err());
    }

    #[test]
    fn test_stop_reason_mapping() {
        let cases = vec![
            (Some("stop"), LlmStopReason::EndTurn),
            (Some("tool_calls"), LlmStopReason::ToolUse),
            (Some("length"), LlmStopReason::MaxTokens),
            (Some("content_filter"), LlmStopReason::ContentFilter),
            (None, LlmStopReason::EndTurn),
            (Some("unknown"), LlmStopReason::EndTurn),
        ];

        for (input, expected) in cases {
            let resp = OpenAiResponse {
                choices: vec![OpenAiChoice {
                    message: OpenAiMessage {
                        role: "assistant".into(),
                        content: Some(OpenAiContent::Text("hi".into())),
                        tool_calls: None,
                        tool_call_id: None,
                    },
                    finish_reason: input.map(String::from),
                }],
                usage: None,
            };
            let llm = from_openai_response(resp).unwrap();
            assert_eq!(llm.stop_reason, expected, "for input {input:?}");
        }
    }

    #[test]
    fn test_openai_request_serialization() {
        let req = OpenAiRequest {
            model: "gpt-4o".into(),
            messages: vec![OpenAiMessage {
                role: "user".into(),
                content: Some(OpenAiContent::Text("Hello".into())),
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: None,
            max_tokens: 4096,
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "gpt-4o");
        assert_eq!(json["max_tokens"], 4096);
        // tools should be omitted when None
        assert!(json.get("tools").is_none());
    }
}
