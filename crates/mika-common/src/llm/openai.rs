use std::sync::LazyLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{Instrument, debug, info, info_span, warn};

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

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    /// OpenAI-standard nested cache metrics (`prompt_tokens_details.cached_tokens`).
    prompt_tokens_details: Option<PromptTokensDetails>,
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

use super::{RETRY_BUFFER_SECS, TRANSPORT_RETRY_MIN_REMAINING_SECS, TYPICAL_CALL_DURATION_SECS};

/// OpenAI-compatible provider that works with OpenAI, Ollama, vLLM, Groq, etc.
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    max_tokens: u32,
    provider_kind: ProviderKind,
    /// When true AND the `telemetry` feature is enabled, attach request/response
    /// bodies as `gen_ai.prompt` / `gen_ai.completion` span attributes. See #671.
    #[cfg_attr(not(feature = "telemetry"), allow(dead_code))]
    log_llm_bodies: bool,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        base_url: String,
        api_key: Option<String>,
        model: String,
        max_tokens: u32,
        provider_kind: ProviderKind,
        log_llm_bodies: bool,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(super::http_timeout_secs()))
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
            log_llm_bodies,
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

        // Dev-mode body logging (gated by MIKA_LOG_LLM_BODIES / mika::llm_debug target)
        if tracing::enabled!(target: "mika::llm_debug", tracing::Level::DEBUG)
            && let Ok(body_json) = serde_json::to_string(request)
        {
            debug!(target: "mika::llm_debug", body = %body_json, provider = %self.provider_kind, "llm request body");
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

        // Dev-mode body logging
        if tracing::enabled!(target: "mika::llm_debug", tracing::Level::DEBUG) {
            debug!(target: "mika::llm_debug", body = ?resp, provider = %self.provider_kind, "llm response body");
        }

        Ok(resp)
    }

    /// Inner implementation of send_message with retry logic and optional deadline.
    async fn send_message_inner(
        &self,
        request: &LlmRequest,
        deadline: Option<Instant>,
    ) -> Result<LlmResponse, LlmError> {
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
                // Deadline-aware retry abort: if the remaining time before the
                // agent deadline cannot fit another LLM call attempt, bail out
                // instead of wasting time on a doomed retry.
                //
                // mika#1744 AC4-primary: transport-class errors (DNS,
                // connection refused, TLS, socket reset) resolve in seconds,
                // not the full 120s per-request timeout. Use a smaller
                // remaining-budget threshold (60s) when the last error was
                // a transport failure. This is what unblocks mika-qa's
                // z.ai wedge — the 2026-07-07 kill happened because the
                // deadline was already ~2min PAST when the transport error
                // hit, and the 120s abort threshold left no room for the
                // fast-failing transport retry.
                if let Some(dl) = deadline {
                    let remaining = dl.saturating_duration_since(Instant::now());
                    let last_was_transport = last_error
                        .as_ref()
                        .is_some_and(super::error::LlmError::is_transport);
                    let threshold_secs = if last_was_transport {
                        TRANSPORT_RETRY_MIN_REMAINING_SECS
                    } else {
                        TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS
                    };
                    if remaining < Duration::from_secs(threshold_secs) {
                        warn!(
                            attempt,
                            remaining_ms = remaining.as_millis() as u64,
                            threshold_secs,
                            last_was_transport,
                            "aborting retry chain — remaining deadline insufficient for another attempt"
                        );
                        break;
                    }
                }

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
                        cache_read_tokens = llm_response.usage.cache_read_input_tokens,
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

        // Distinguish deadline-abort from normal retry exhaustion for
        // diagnostics. Mirrors the retry-loop's transport-aware threshold
        // (mika#1744) so the two branches agree on which errors classify
        // as "aborted by deadline" — otherwise a transport-late failure
        // would classify as "max retries exceeded" instead of the more
        // accurate "deadline budget insufficient" surface.
        let last_was_transport = last_error
            .as_ref()
            .is_some_and(super::error::LlmError::is_transport);
        let deadline_threshold_secs = if last_was_transport {
            TRANSPORT_RETRY_MIN_REMAINING_SECS
        } else {
            TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS
        };
        let deadline_aborted = deadline.is_some_and(|dl| {
            dl.saturating_duration_since(Instant::now())
                < Duration::from_secs(deadline_threshold_secs)
        });

        Err(last_error.unwrap_or_else(|| {
            if deadline_aborted {
                LlmError::ProviderError("retry chain aborted: deadline budget insufficient".into())
            } else {
                LlmError::ProviderError("max retries exceeded".into())
            }
        }))
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        self.send_message_with_deadline(request, None).await
    }

    async fn send_message_with_deadline(
        &self,
        request: &LlmRequest,
        deadline: Option<Instant>,
    ) -> Result<LlmResponse, LlmError> {
        let span = info_span!(
            target: "mika::otel",
            "llm_call",
            model = %request.model,
            max_tokens = request.max_tokens,
            provider = %self.provider_kind,
        );

        // Set gen_ai semantic convention attributes for Langfuse generation classification
        #[cfg(feature = "telemetry")]
        {
            use tracing_opentelemetry::OpenTelemetrySpanExt;
            span.set_attribute("gen_ai.operation.name", "chat");
            span.set_attribute("gen_ai.provider.name", self.provider_kind.to_string());
            span.set_attribute("gen_ai.request.model", request.model.clone());
            span.set_attribute("gen_ai.request.max_tokens", request.max_tokens as i64);

            // Attach request body for Langfuse Generation "Input" (#671)
            if self.log_llm_bodies {
                let openai_req = to_openai_request(request);
                if let Ok(body_json) = serde_json::to_string(&openai_req) {
                    span.set_attribute(
                        "gen_ai.prompt",
                        super::truncate_chars(&body_json, super::MAX_RESPONSE_TEXT_CHARS),
                    );
                }
            }
        }

        let response = self
            .send_message_inner(request, deadline)
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

            // Attach response body for Langfuse Generation "Output" (#671)
            if self.log_llm_bodies
                && let Some(text) = super::serialize_response_text(
                    &response.content,
                    super::MAX_RESPONSE_TEXT_CHARS,
                )
            {
                span.set_attribute("gen_ai.completion", text);
            }
        }

        Ok(response)
    }

    fn provider_name(&self) -> &str {
        self.provider_kind.config_prefix()
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
        matches!(
            self.provider_kind,
            ProviderKind::OpenAi
                | ProviderKind::OpenRouter
                | ProviderKind::Mistral
                | ProviderKind::Google
                | ProviderKind::DeepSeek
        )
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
                            is_error,
                            ..
                        } => {
                            let mut text = match content {
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
                            // OpenAI has no first-class is_error field; prefix the
                            // content so the model knows the tool call failed.
                            if *is_error {
                                text = format!("[ERROR] {text}");
                            }
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

    // Extract structured tool calls
    let has_structured_tool_calls = choice
        .message
        .tool_calls
        .as_ref()
        .is_some_and(|tc| !tc.is_empty());
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

    // Extract XML-formatted tool calls from text content (e.g. <function=name>...</function>).
    // Only run when no structured tool_calls were returned — prevents duplicates.
    if !has_structured_tool_calls {
        let mut new_content = Vec::new();
        let mut xml_tool_calls = Vec::new();
        for item in content {
            if let LlmResponseContent::Text(ref text) = item {
                let (extracted, remaining) = extract_xml_tool_calls(text);
                if !extracted.is_empty() {
                    info!(
                        extracted_count = extracted.len(),
                        "extracted XML-formatted tool calls from text response"
                    );
                    xml_tool_calls.extend(extracted);
                    if !remaining.is_empty() {
                        new_content.push(LlmResponseContent::Text(remaining));
                    }
                } else {
                    new_content.push(item);
                }
            } else {
                new_content.push(item);
            }
        }
        new_content.extend(xml_tool_calls);
        content = new_content;
    }

    // Determine stop_reason. If content contains tool calls (structured or XML-extracted)
    // but finish_reason didn't indicate tool use, override to ToolUse.
    let has_any_tool_calls = content
        .iter()
        .any(|c| matches!(c, LlmResponseContent::ToolCall { .. }));
    let stop_reason = match choice.finish_reason.as_deref() {
        Some("tool_calls") => LlmStopReason::ToolUse,
        Some("length") => LlmStopReason::MaxTokens,
        Some("content_filter") => LlmStopReason::ContentFilter,
        _ if has_any_tool_calls => LlmStopReason::ToolUse,
        _ => LlmStopReason::EndTurn,
    };

    let usage = resp.usage.map_or_else(LlmUsage::default, |u| {
        let cache_read = u
            .prompt_tokens_details
            .map(|d| d.cached_tokens)
            .filter(|&t| t > 0);
        LlmUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: cache_read,
        }
    });

    // Extract <think>…</think> blocks as reasoning (e.g. MiniMax, DeepSeek).
    let mut reasoning: Option<String> = None;
    for item in &mut content {
        if let LlmResponseContent::Text(text) = item
            && let Some((think_text, stripped)) = extract_think_block(text)
        {
            reasoning = Some(think_text);
            *text = stripped;
        }
    }
    // Remove empty text entries left after stripping
    content.retain(|c| !matches!(c, LlmResponseContent::Text(t) if t.is_empty()));

    Ok(LlmResponse {
        content,
        reasoning,
        stop_reason,
        usage,
    })
}

/// Extract `<think>…</think>` block from text, returning (thinking, remaining_text).
/// Returns `None` if no think block is found.
///
/// Used by both `OpenAiCompatibleProvider` and `OllamaProvider` for models that
/// emit reasoning in `<think>` tags (e.g., DeepSeek-R1, MiniMax thinking models).
pub(crate) fn extract_think_block(text: &str) -> Option<(String, String)> {
    let start_tag = "<think>";
    let end_tag = "</think>";
    let start = text.find(start_tag)?;
    let end = text.find(end_tag)?;
    if end <= start {
        return None;
    }
    let think_content = text[start + start_tag.len()..end].trim().to_string();
    let remaining = format!("{}{}", &text[..start], &text[end + end_tag.len()..])
        .trim()
        .to_string();
    if think_content.is_empty() {
        return None;
    }
    Some((think_content, remaining))
}

// Regex for `<tool_call>...<function=name>args</function>...</tool_call>` or
// `<tool_call>{"name":"...","arguments":{...}}</tool_call>`.
static RE_WRAPPED_TOOL_CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<tool_call>\s*(?:<function=([^>]+)>([\s\S]*?)</function>|(\{[\s\S]*?\}))\s*</tool_call>").unwrap()
});

// Regex for bare `<function=name>args</function>` (no `<tool_call>` wrapper).
static RE_BARE_FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<function=([^>]+)>([\s\S]*?)</function>").unwrap());

/// Extract XML-formatted tool calls from text content.
///
/// Some OpenAI-compatible providers emit tool calls as XML text
/// (e.g. `<function=search>{"query":"test"}</function>`) instead of structured
/// `tool_calls` in the response. This function extracts those, converts them to
/// `LlmResponseContent::ToolCall` items, and returns the remaining text with
/// XML removed.
///
/// Returns `(extracted_tool_calls, remaining_text)`.
fn extract_xml_tool_calls(text: &str) -> (Vec<LlmResponseContent>, String) {
    if !text.contains('<') {
        return (vec![], text.to_string());
    }

    let mut tool_calls = Vec::new();
    let mut remaining = text.to_string();
    let mut call_index: usize = 0;

    // Pass 1: Extract wrapped `<tool_call>...</tool_call>` blocks.
    // Use captures() directly to avoid running the regex twice per iteration.
    while let Some(caps) = RE_WRAPPED_TOOL_CALL.captures(&remaining) {
        let full_match = caps.get(0).unwrap();
        let start = full_match.start();
        let end = full_match.end();

        // Determine if it's a <function=name>args</function> or bare JSON inside <tool_call>.
        if let Some(name_match) = caps.get(1) {
            // <tool_call><function=name>args</function></tool_call>
            let name = name_match.as_str().to_string();
            let raw_args = caps.get(2).map_or("", |m| m.as_str()).trim();
            let arguments = parse_tool_arguments(raw_args);
            tool_calls.push(LlmResponseContent::ToolCall {
                id: format!("xml_call_{call_index}"),
                name,
                arguments,
            });
        } else if let Some(json_match) = caps.get(3) {
            // <tool_call>{"name":"...","arguments":{...}}</tool_call>
            if let Some(tc) = parse_json_tool_call(json_match.as_str(), call_index) {
                tool_calls.push(tc);
            } else {
                // Malformed JSON — skip this match by removing it to avoid infinite loop.
                call_index += 1;
                remaining = format!("{}{}", &remaining[..start], &remaining[end..]);
                continue;
            }
        }

        call_index += 1;
        remaining = format!("{}{}", &remaining[..start], &remaining[end..]);
    }

    // Pass 2: Extract bare `<function=name>args</function>` (not already consumed).
    while let Some(caps) = RE_BARE_FUNCTION.captures(&remaining) {
        let full_match = caps.get(0).unwrap();
        let start = full_match.start();
        let end = full_match.end();
        let name = caps.get(1).unwrap().as_str().to_string();
        let raw_args = caps.get(2).map_or("", |m| m.as_str()).trim();
        let arguments = parse_tool_arguments(raw_args);
        tool_calls.push(LlmResponseContent::ToolCall {
            id: format!("xml_call_{call_index}"),
            name,
            arguments,
        });
        call_index += 1;
        remaining = format!("{}{}", &remaining[..start], &remaining[end..]);
    }

    // Clean up whitespace left by removals.
    let remaining = remaining.trim().to_string();

    (tool_calls, remaining)
}

/// Parse tool call arguments: valid JSON object → Value, empty → empty object,
/// invalid → Value::String (same fallback as structured tool_calls).
fn parse_tool_arguments(raw: &str) -> Value {
    if raw.is_empty() {
        return Value::Object(serde_json::Map::new());
    }
    serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

/// Parse a JSON-formatted tool call: `{"name":"func","arguments":{...}}`.
fn parse_json_tool_call(json_str: &str, index: usize) -> Option<LlmResponseContent> {
    let obj: Value = serde_json::from_str(json_str).ok()?;
    let name = obj.get("name")?.as_str()?.to_string();
    let arguments = obj
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    Some(LlmResponseContent::ToolCall {
        id: format!("xml_call_{index}"),
        name,
        arguments,
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
                prompt_tokens_details: None,
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

    #[test]
    fn test_extract_think_block_basic() {
        let (thinking, remaining) =
            extract_think_block("<think>Let me think...</think>Hello!").unwrap();
        assert_eq!(thinking, "Let me think...");
        assert_eq!(remaining, "Hello!");
    }

    #[test]
    fn test_extract_think_block_with_newlines() {
        let input = "<think>\nI should respond friendly.\n</think>\n\nHey Vincent!";
        let (thinking, remaining) = extract_think_block(input).unwrap();
        assert_eq!(thinking, "I should respond friendly.");
        assert_eq!(remaining, "Hey Vincent!");
    }

    #[test]
    fn test_extract_think_block_no_tags() {
        assert!(extract_think_block("Just a normal response").is_none());
    }

    #[test]
    fn test_extract_think_block_empty_think() {
        assert!(extract_think_block("<think></think>Hello").is_none());
    }

    #[test]
    fn test_from_openai_response_strips_think_tags() {
        let resp = OpenAiResponse {
            choices: vec![OpenAiChoice {
                message: OpenAiMessage {
                    role: "assistant".into(),
                    content: Some(OpenAiContent::Text(
                        "<think>\nLet me think about this.\n</think>\n\nHello!".into(),
                    )),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: Some(OpenAiUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                prompt_tokens_details: None,
            }),
        };

        let llm = from_openai_response(resp).unwrap();
        assert_eq!(llm.text(), "Hello!");
        assert_eq!(llm.reasoning, Some("Let me think about this.".into()));
    }

    #[test]
    fn test_from_openai_response_no_think_tags_unchanged() {
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
            usage: None,
        };

        let llm = from_openai_response(resp).unwrap();
        assert_eq!(llm.text(), "Hello!");
        assert!(llm.reasoning.is_none());
    }

    // -- extract_xml_tool_calls tests --

    #[test]
    fn test_extract_xml_tool_calls_wrapped_function_tag() {
        let text = r#"<tool_call>
<function=search_memory>
{"query": "meetings"}
</function>
</tool_call>"#;
        let (calls, remaining) = extract_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        if let LlmResponseContent::ToolCall {
            id,
            name,
            arguments,
        } = &calls[0]
        {
            assert_eq!(id, "xml_call_0");
            assert_eq!(name, "search_memory");
            assert_eq!(arguments, &json!({"query": "meetings"}));
        } else {
            panic!("expected ToolCall");
        }
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_extract_xml_tool_calls_bare_function_tag() {
        let text = r#"<function=list_tasks>
{"status": "active"}
</function>"#;
        let (calls, remaining) = extract_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        if let LlmResponseContent::ToolCall {
            name, arguments, ..
        } = &calls[0]
        {
            assert_eq!(name, "list_tasks");
            assert_eq!(arguments, &json!({"status": "active"}));
        } else {
            panic!("expected ToolCall");
        }
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_extract_xml_tool_calls_json_in_tool_call() {
        let text = r#"<tool_call>
{"name": "search_memory", "arguments": {"query": "test"}}
</tool_call>"#;
        let (calls, remaining) = extract_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        if let LlmResponseContent::ToolCall {
            name, arguments, ..
        } = &calls[0]
        {
            assert_eq!(name, "search_memory");
            assert_eq!(arguments, &json!({"query": "test"}));
        } else {
            panic!("expected ToolCall");
        }
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_extract_xml_tool_calls_empty_arguments() {
        let text = "<function=list_tasks></function>";
        let (calls, remaining) = extract_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        if let LlmResponseContent::ToolCall {
            name, arguments, ..
        } = &calls[0]
        {
            assert_eq!(name, "list_tasks");
            assert_eq!(arguments, &json!({}));
        } else {
            panic!("expected ToolCall");
        }
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_extract_xml_tool_calls_multiple() {
        let text = r#"<function=search_memory>{"query": "a"}</function>
<function=store_fact>{"text": "b"}</function>"#;
        let (calls, remaining) = extract_xml_tool_calls(text);
        assert_eq!(calls.len(), 2);
        if let LlmResponseContent::ToolCall { name, .. } = &calls[0] {
            assert_eq!(name, "search_memory");
        }
        if let LlmResponseContent::ToolCall { name, .. } = &calls[1] {
            assert_eq!(name, "store_fact");
        }
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_extract_xml_tool_calls_mixed_text() {
        let text = "Let me search for that.\n<function=search_memory>{\"query\": \"test\"}</function>\nHere are the results.";
        let (calls, remaining) = extract_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert!(remaining.contains("Let me search for that."));
        assert!(remaining.contains("Here are the results."));
        assert!(!remaining.contains("<function="));
    }

    #[test]
    fn test_extract_xml_tool_calls_no_xml() {
        let text = "This is plain text with no tool calls.";
        let (calls, remaining) = extract_xml_tool_calls(text);
        assert!(calls.is_empty());
        assert_eq!(remaining, text);
    }

    #[test]
    fn test_extract_xml_tool_calls_malformed() {
        // Missing closing tag — should not be extracted.
        let text = "<function=search_memory>{\"query\": \"test\"}";
        let (calls, remaining) = extract_xml_tool_calls(text);
        assert!(calls.is_empty());
        assert_eq!(remaining, text);
    }

    #[test]
    fn test_extract_xml_tool_calls_with_think_block() {
        let text = "<think>I should search</think><function=search_memory>{\"query\": \"test\"}</function>";
        let (calls, remaining) = extract_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        // Think block should remain in the text (handled separately by extract_think_block).
        assert!(remaining.contains("<think>"));
    }

    #[test]
    fn test_from_openai_response_xml_tool_calls() {
        let resp = OpenAiResponse {
            choices: vec![OpenAiChoice {
                message: OpenAiMessage {
                    role: "assistant".into(),
                    content: Some(OpenAiContent::Text(
                        "<function=search_memory>\n{\"query\": \"test\"}\n</function>".into(),
                    )),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: None,
        };

        let llm = from_openai_response(resp).unwrap();
        assert!(llm.has_tool_calls());
        assert_eq!(llm.stop_reason, LlmStopReason::ToolUse);
        let tc = llm.tool_calls();
        assert_eq!(tc.len(), 1);
        if let LlmResponseContent::ToolCall { name, .. } = tc[0] {
            assert_eq!(name, "search_memory");
        }
    }

    #[test]
    fn test_from_openai_response_xml_skipped_when_structured() {
        let resp = OpenAiResponse {
            choices: vec![OpenAiChoice {
                message: OpenAiMessage {
                    role: "assistant".into(),
                    content: Some(OpenAiContent::Text(
                        "<function=search_memory>{\"query\": \"stale\"}</function>".into(),
                    )),
                    tool_calls: Some(vec![OpenAiToolCall {
                        id: "call_123".into(),
                        call_type: "function".into(),
                        function: OpenAiFunction {
                            name: "search_memory".into(),
                            arguments: "{\"query\": \"real\"}".into(),
                        },
                    }]),
                    tool_call_id: None,
                },
                finish_reason: Some("tool_calls".into()),
            }],
            usage: None,
        };

        let llm = from_openai_response(resp).unwrap();
        // Should have exactly 1 tool call (the structured one), not 2.
        let tc = llm.tool_calls();
        assert_eq!(tc.len(), 1);
        if let LlmResponseContent::ToolCall { id, .. } = tc[0] {
            assert_eq!(id, "call_123");
        }
        // The text content should still have the XML (it was not extracted).
        assert_eq!(
            llm.text(),
            "<function=search_memory>{\"query\": \"stale\"}</function>"
        );
    }

    #[test]
    fn test_from_openai_response_stop_reason_flip() {
        // finish_reason is "stop" but XML tool calls are extracted — stop_reason should be ToolUse.
        let resp = OpenAiResponse {
            choices: vec![OpenAiChoice {
                message: OpenAiMessage {
                    role: "assistant".into(),
                    content: Some(OpenAiContent::Text(
                        "<tool_call>\n<function=list_tasks>\n{}\n</function>\n</tool_call>".into(),
                    )),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: None,
        };

        let llm = from_openai_response(resp).unwrap();
        assert_eq!(llm.stop_reason, LlmStopReason::ToolUse);
        assert!(llm.has_tool_calls());
    }

    // -- Cache token parsing tests (#479) --

    fn make_response_with_usage(usage: Option<OpenAiUsage>) -> OpenAiResponse {
        OpenAiResponse {
            choices: vec![OpenAiChoice {
                message: OpenAiMessage {
                    role: "assistant".into(),
                    content: Some(OpenAiContent::Text("Hello!".into())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage,
        }
    }

    #[test]
    fn test_from_openai_response_cache_hit() {
        let resp = make_response_with_usage(Some(OpenAiUsage {
            prompt_tokens: 36000,
            completion_tokens: 500,
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: 8192,
            }),
        }));

        let llm = from_openai_response(resp).unwrap();
        assert_eq!(llm.usage.input_tokens, 36000);
        assert_eq!(llm.usage.output_tokens, 500);
        assert_eq!(llm.usage.cache_read_input_tokens, Some(8192));
        assert_eq!(llm.usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn test_from_openai_response_no_cache_details() {
        let resp = make_response_with_usage(Some(OpenAiUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            prompt_tokens_details: None,
        }));

        let llm = from_openai_response(resp).unwrap();
        assert_eq!(llm.usage.cache_read_input_tokens, None);
        assert_eq!(llm.usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn test_from_openai_response_zero_or_default_cached_tokens() {
        // Zero cached_tokens (explicit or via #[serde(default)] for empty `{}`) maps to None.
        let resp = make_response_with_usage(Some(OpenAiUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            prompt_tokens_details: Some(PromptTokensDetails { cached_tokens: 0 }),
        }));

        let llm = from_openai_response(resp).unwrap();
        assert_eq!(llm.usage.cache_read_input_tokens, None);
    }

    #[test]
    fn test_from_openai_response_cache_details_json_deserialization() {
        // End-to-end: verify serde deserialization from a real OpenRouter-style JSON payload
        let json = r#"{
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello!"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 36000,
                "completion_tokens": 500,
                "prompt_tokens_details": {
                    "cached_tokens": 8192
                }
            }
        }"#;

        let resp: OpenAiResponse = serde_json::from_str(json).unwrap();
        let llm = from_openai_response(resp).unwrap();
        assert_eq!(llm.usage.cache_read_input_tokens, Some(8192));
    }
}
