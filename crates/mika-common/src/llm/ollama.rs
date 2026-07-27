use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{Instrument, debug, error, info, info_span, warn};

use super::error::LlmError;
use super::openai::extract_think_block;
use super::types::*;
use super::{LlmProvider, RETRY_BUFFER_SECS, TYPICAL_CALL_DURATION_SECS};

/// One-shot debug-flag-gated payload dump (mika#1387).
///
/// When `MIKA_OLLAMA_DUMP_PAYLOAD=<path>` is set, the NEXT `/api/chat`
/// request body through `OllamaProvider` is written to that path, then the
/// flag flips and subsequent requests are no-ops. Process-scoped — resets
/// only on process restart.
///
/// Bounded at 256 KiB. Overflow appends a truncation marker so the file
/// stays grep-able and the operator notices.
const PAYLOAD_DUMP_CAP_BYTES: usize = 256 * 1024;
static PAYLOAD_DUMP_FIRED: AtomicBool = AtomicBool::new(false);

/// Test-only handle that resets the one-shot flag so each test starts fresh.
#[cfg(test)]
pub(crate) fn reset_payload_dump_flag() {
    PAYLOAD_DUMP_FIRED.store(false, Ordering::Relaxed);
}

/// If `MIKA_OLLAMA_DUMP_PAYLOAD` is set and the one-shot flag hasn't fired
/// this process, write `body_json` (capped at `PAYLOAD_DUMP_CAP_BYTES`) to
/// the configured path. Failure resets the flag so the operator can retry
/// after fixing the path; success flips it permanently.
fn try_dump_payload(body_json: &str) {
    let path = match std::env::var("MIKA_OLLAMA_DUMP_PAYLOAD") {
        Ok(p) if !p.is_empty() => p,
        _ => return,
    };

    // Atomically reserve the one-shot slot.
    if PAYLOAD_DUMP_FIRED.swap(true, Ordering::Relaxed) {
        return;
    }

    let total_len = body_json.len();
    let (slice, truncated) = if total_len > PAYLOAD_DUMP_CAP_BYTES {
        (&body_json[..PAYLOAD_DUMP_CAP_BYTES], true)
    } else {
        (body_json, false)
    };

    let mut content = String::with_capacity(slice.len() + 128);
    content.push_str(slice);
    if truncated {
        content.push_str(&format!(
            "\n<!-- TRUNCATED at {PAYLOAD_DUMP_CAP_BYTES} bytes; total payload was {total_len} bytes -->\n"
        ));
    }

    match std::fs::write(&path, &content) {
        Ok(()) => warn!(
            path = %path,
            bytes_written = content.len(),
            truncated,
            "Ollama payload dumped (one-shot)"
        ),
        Err(e) => {
            // Reset so the operator can retry after fixing the path.
            PAYLOAD_DUMP_FIRED.store(false, Ordering::Relaxed);
            error!(path = %path, error = %e, "Ollama payload dump failed");
        }
    }
}

// -- Ollama wire types --

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    options: OllamaOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OllamaTool>>,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
    /// Tool calls from assistant messages (echoed back in conversation history).
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Serialize)]
struct OllamaOptions {
    num_predict: u32,
}

/// Tool definition sent in the request (same shape as OpenAI).
#[derive(Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OllamaFunctionDef,
}

#[derive(Serialize)]
struct OllamaFunctionDef {
    name: String,
    description: String,
    parameters: Value,
}

/// Tool call returned in the response message.
#[derive(Debug, Serialize, Deserialize)]
struct OllamaToolCall {
    function: OllamaFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    /// Ollama returns arguments as a JSON object, not a string (unlike OpenAI).
    arguments: Value,
}

// -- Response types --

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    #[allow(dead_code)]
    model: String,
    message: OllamaResponseMessage,
    #[allow(dead_code)]
    done: bool,
    #[allow(dead_code)]
    total_duration: Option<u64>,
    eval_count: Option<u64>,
    prompt_eval_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

// -- Error response --

#[derive(Deserialize)]
struct OllamaErrorResponse {
    error: String,
}

// -- Provider implementation --

const MAX_RETRIES: u32 = 3;

/// Native Ollama provider that uses `/api/chat` (Ollama's native endpoint).
///
/// Unlike the OpenAI-compatible adapter, this provider:
/// - Uses Ollama's native request/response format
/// - Does not require `/v1` suffix in the base URL
/// - Uses `/api/tags` for health checks
/// - Does not support vision (deferred)
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    max_tokens: u32,
    #[cfg_attr(not(feature = "telemetry"), allow(dead_code))]
    log_llm_bodies: bool,
    /// The provider kind this instance was created for. Distinguishes
    /// `ProviderKind::Ollama` from `ProviderKind::MikaModel` (both use the
    /// same Ollama transport but report different `provider_name()`).
    provider_kind: super::ProviderKind,
}

impl OllamaProvider {
    pub fn new(
        base_url: String,
        api_key: Option<String>,
        model: String,
        max_tokens: u32,
        log_llm_bodies: bool,
    ) -> Self {
        Self::with_provider_kind(
            base_url,
            api_key,
            model,
            max_tokens,
            log_llm_bodies,
            super::ProviderKind::Ollama,
        )
    }

    /// Create an `OllamaProvider` with an explicit `ProviderKind`.
    ///
    /// Used by `ProviderKind::MikaModel` so the instance reports
    /// `"mikamodel"` from `provider_name()` while sharing the same transport.
    pub fn with_provider_kind(
        base_url: String,
        api_key: Option<String>,
        model: String,
        max_tokens: u32,
        log_llm_bodies: bool,
        provider_kind: super::ProviderKind,
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
            log_llm_bodies,
            provider_kind,
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/api/chat", self.base_url)
    }

    fn tags_url(&self) -> String {
        format!("{}/api/tags", self.base_url)
    }

    fn to_ollama_request(&self, request: &LlmRequest) -> OllamaChatRequest {
        let mut messages = Vec::new();

        // System prompt goes as first message with role "system"
        if let Some(ref system) = request.system {
            messages.push(OllamaMessage {
                role: "system".into(),
                content: system.clone(),
                tool_calls: None,
            });
        }

        // Convert conversation messages
        for msg in &request.messages {
            match msg.role {
                LlmRole::User => {
                    let content = match &msg.content {
                        LlmContent::Text(t) => t.clone(),
                        LlmContent::Blocks(blocks) => blocks
                            .iter()
                            .filter_map(|b| match b {
                                LlmContentBlock::Text(t) => Some(t.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(""),
                    };
                    messages.push(OllamaMessage {
                        role: "user".into(),
                        content,
                        tool_calls: None,
                    });
                }
                LlmRole::Assistant => {
                    let mut text_parts = Vec::new();
                    let mut tool_calls = Vec::new();

                    match &msg.content {
                        LlmContent::Text(t) => text_parts.push(t.as_str()),
                        LlmContent::Blocks(blocks) => {
                            for block in blocks {
                                match block {
                                    LlmContentBlock::Text(t) => text_parts.push(t.as_str()),
                                    LlmContentBlock::ToolCall {
                                        name, arguments, ..
                                    } => {
                                        tool_calls.push(OllamaToolCall {
                                            function: OllamaFunctionCall {
                                                name: name.clone(),
                                                arguments: arguments.clone(),
                                            },
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        }
                    };

                    messages.push(OllamaMessage {
                        role: "assistant".into(),
                        content: text_parts.join(""),
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                    });
                }
                LlmRole::Tool => match &msg.content {
                    LlmContent::Blocks(blocks) => {
                        for block in blocks {
                            if let LlmContentBlock::ToolResult { content, .. } = block {
                                let text = match content {
                                    LlmToolResultContent::Text(t) => t.clone(),
                                    LlmToolResultContent::Blocks(parts) => parts
                                        .iter()
                                        .filter_map(|p| match p {
                                            LlmToolResultBlock::Text(t) => Some(t.as_str()),
                                            _ => None,
                                        })
                                        .collect::<Vec<_>>()
                                        .join(""),
                                };
                                messages.push(OllamaMessage {
                                    role: "tool".into(),
                                    content: text,
                                    tool_calls: None,
                                });
                            }
                        }
                    }
                    LlmContent::Text(t) => {
                        messages.push(OllamaMessage {
                            role: "tool".into(),
                            content: t.clone(),
                            tool_calls: None,
                        });
                    }
                },
            }
        }

        // Convert tools
        let tools = request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|t| OllamaTool {
                    tool_type: "function".into(),
                    function: OllamaFunctionDef {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: t.parameters.clone(),
                    },
                })
                .collect()
        });

        OllamaChatRequest {
            model: request.model.clone(),
            messages,
            stream: false,
            options: OllamaOptions {
                num_predict: self.max_tokens,
            },
            tools,
        }
    }

    fn from_ollama_response(response: OllamaChatResponse) -> LlmResponse {
        let mut text = response.message.content;

        // Extract <think>…</think> blocks as reasoning (e.g., DeepSeek-R1 via Ollama)
        let reasoning = if let Some((think_text, stripped)) = extract_think_block(&text) {
            text = stripped;
            Some(think_text)
        } else {
            None
        };

        let mut content = Vec::new();

        if !text.is_empty() {
            content.push(LlmResponseContent::Text(text));
        }

        // Parse tool calls from response. Ollama doesn't provide tool call IDs,
        // so we generate positional IDs (unique within a single response turn).
        let has_tool_calls = response
            .message
            .tool_calls
            .as_ref()
            .is_some_and(|tc| !tc.is_empty());

        if let Some(tool_calls) = response.message.tool_calls {
            for (index, tc) in tool_calls.into_iter().enumerate() {
                content.push(LlmResponseContent::ToolCall {
                    id: format!("ollama_tc_{index}"),
                    name: tc.function.name,
                    arguments: tc.function.arguments,
                });
            }
        }

        let usage = LlmUsage {
            input_tokens: response.prompt_eval_count.unwrap_or(0),
            output_tokens: response.eval_count.unwrap_or(0),
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };

        LlmResponse {
            content,
            reasoning,
            stop_reason: if has_tool_calls {
                LlmStopReason::ToolUse
            } else {
                LlmStopReason::EndTurn
            },
            usage,
        }
    }

    async fn send_once(&self, request: &OllamaChatRequest) -> Result<OllamaChatResponse, LlmError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        // Include auth header only if api_key is set (ollama typically runs unauthenticated)
        if let Some(ref key) = self.api_key {
            let auth = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|e| LlmError::ProviderError(format!("invalid API key: {e}")))?;
            headers.insert(AUTHORIZATION, auth);
        }

        // Serialize once for both the dev-mode debug log and the one-shot
        // payload dump (#1387). Either gate may be enabled independently;
        // if both are off, the serialize cost is skipped.
        let dump_enabled =
            std::env::var_os("MIKA_OLLAMA_DUMP_PAYLOAD").is_some_and(|v| !v.is_empty());
        let debug_log_enabled = tracing::enabled!(target: "mika::llm_debug", tracing::Level::DEBUG);
        if dump_enabled || debug_log_enabled {
            match serde_json::to_string(request) {
                Ok(body_json) => {
                    if debug_log_enabled {
                        debug!(target: "mika::llm_debug", body = %body_json, provider = "ollama", "llm request body");
                    }
                    if dump_enabled {
                        try_dump_payload(&body_json);
                    }
                }
                Err(e) => {
                    warn!(error = %e, "ollama: failed to serialize request for debug log/dump");
                }
            }
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
            let message = serde_json::from_str::<OllamaErrorResponse>(&body)
                .map(|e| e.error)
                .unwrap_or_else(|_| {
                    let truncated: String = body.chars().take(200).collect();
                    format!("HTTP {status_code}: {truncated}")
                });
            warn!(status = status_code, error_message = %message, "Ollama API error");
            // HTTP 500 is retryable (covers model loading delays)
            let retryable = matches!(status_code, 429 | 500 | 503);
            return Err(LlmError::HttpError {
                status: status_code,
                message,
                retryable,
            });
        }

        let resp: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| LlmError::ParseError(format!("failed to parse ollama response: {e}")))?;

        // Dev-mode body logging
        if tracing::enabled!(target: "mika::llm_debug", tracing::Level::DEBUG) {
            debug!(target: "mika::llm_debug", body = ?resp, provider = "ollama", "llm response body");
        }

        Ok(resp)
    }

    /// Inner implementation of send_message with retry logic and optional deadline.
    async fn send_message_inner(
        &self,
        request: &LlmRequest,
        deadline: Option<Instant>,
    ) -> Result<LlmResponse, LlmError> {
        let ollama_request = self.to_ollama_request(request);

        info!(
            model = %request.model,
            max_tokens = request.max_tokens,
            provider = "ollama",
            "llm_call started"
        );

        let mut last_error = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                // Deadline-aware retry abort
                if let Some(dl) = deadline {
                    let remaining = dl.saturating_duration_since(Instant::now());
                    if remaining
                        < Duration::from_secs(TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS)
                    {
                        warn!(
                            attempt,
                            remaining_ms = remaining.as_millis() as u64,
                            "aborting retry chain — remaining deadline insufficient for another attempt"
                        );
                        break;
                    }
                }

                let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                warn!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    "retrying Ollama API call"
                );
                tokio::time::sleep(delay).await;
            }

            match self.send_once(&ollama_request).await {
                Ok(response) => {
                    let llm_response = Self::from_ollama_response(response);
                    info!(
                        model = %request.model,
                        input_tokens = llm_response.usage.input_tokens,
                        output_tokens = llm_response.usage.output_tokens,
                        stop_reason = ?llm_response.stop_reason,
                        provider = "ollama",
                        "llm_call completed"
                    );
                    return Ok(llm_response);
                }
                Err(e) => {
                    if attempt < MAX_RETRIES && e.is_retryable() {
                        warn!(attempt, error = %e, "transient Ollama API error");
                        last_error = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        // Distinguish deadline-abort from normal retry exhaustion
        let deadline_aborted = deadline.is_some_and(|dl| {
            dl.saturating_duration_since(Instant::now())
                < Duration::from_secs(TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS)
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
impl LlmProvider for OllamaProvider {
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
            provider = "ollama",
        );

        // Set gen_ai semantic convention attributes for Langfuse generation classification
        #[cfg(feature = "telemetry")]
        {
            use tracing_opentelemetry::OpenTelemetrySpanExt;
            span.set_attribute("gen_ai.operation.name", "chat");
            span.set_attribute("gen_ai.provider.name", "ollama");
            span.set_attribute("gen_ai.request.model", request.model.clone());
            span.set_attribute("gen_ai.request.max_tokens", request.max_tokens as i64);

            if self.log_llm_bodies {
                let ollama_req = self.to_ollama_request(request);
                if let Ok(body_json) = serde_json::to_string(&ollama_req) {
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
        false
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
            .get(self.tags_url())
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(LlmError::HttpError {
                status: response.status().as_u16(),
                message: "ollama health check failed".into(),
                retryable: false,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_provider() -> OllamaProvider {
        OllamaProvider::new(
            "http://localhost:11434".into(),
            None,
            "llama3".into(),
            4096,
            false,
        )
    }

    // -- URL construction --

    #[test]
    fn test_chat_url() {
        let provider = make_provider();
        assert_eq!(provider.chat_url(), "http://localhost:11434/api/chat");
    }

    #[test]
    fn test_tags_url() {
        let provider = make_provider();
        assert_eq!(provider.tags_url(), "http://localhost:11434/api/tags");
    }

    #[test]
    fn test_url_with_trailing_slash() {
        let provider = OllamaProvider::new(
            "http://localhost:11434/".into(),
            None,
            "llama3".into(),
            4096,
            false,
        );
        assert_eq!(provider.chat_url(), "http://localhost:11434/api/chat");
    }

    // -- Request conversion --

    #[test]
    fn test_to_ollama_request_basic() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: Some("You are helpful.".into()),
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Text("Hello".into()),
            }],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        assert_eq!(ollama_req.model, "llama3");
        assert!(!ollama_req.stream);
        assert_eq!(ollama_req.options.num_predict, 4096);
        assert_eq!(ollama_req.messages.len(), 2); // system + user
        assert_eq!(ollama_req.messages[0].role, "system");
        assert_eq!(ollama_req.messages[0].content, "You are helpful.");
        assert_eq!(ollama_req.messages[1].role, "user");
        assert_eq!(ollama_req.messages[1].content, "Hello");
    }

    #[test]
    fn test_to_ollama_request_no_system() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: None,
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Text("Hi".into()),
            }],
            tools: None,
            max_tokens: 2048,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        assert_eq!(ollama_req.messages.len(), 1);
        assert_eq!(ollama_req.messages[0].role, "user");
    }

    #[test]
    fn test_to_ollama_request_conversation() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: None,
            messages: vec![
                LlmMessage {
                    role: LlmRole::User,
                    content: LlmContent::Text("Hello".into()),
                },
                LlmMessage {
                    role: LlmRole::Assistant,
                    content: LlmContent::Text("Hi there!".into()),
                },
                LlmMessage {
                    role: LlmRole::User,
                    content: LlmContent::Text("How are you?".into()),
                },
            ],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        assert_eq!(ollama_req.messages.len(), 3);
        assert_eq!(ollama_req.messages[0].role, "user");
        assert_eq!(ollama_req.messages[1].role, "assistant");
        assert_eq!(ollama_req.messages[2].role, "user");
    }

    #[test]
    fn test_to_ollama_request_with_tools() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: None,
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Text("Search".into()),
            }],
            tools: Some(vec![LlmToolDefinition {
                name: "search".into(),
                description: "Search the web".into(),
                parameters: json!({"type": "object", "properties": {"query": {"type": "string"}}}),
            }]),
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        let json = serde_json::to_value(&ollama_req).unwrap();
        let tools = json.get("tools").expect("tools should be present");
        assert_eq!(tools.as_array().unwrap().len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "search");
        assert_eq!(tools[0]["function"]["description"], "Search the web");
        assert!(tools[0]["function"]["parameters"]["properties"]["query"].is_object());
    }

    #[test]
    fn test_to_ollama_request_no_tools_omitted() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: None,
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Text("Hello".into()),
            }],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        let json = serde_json::to_value(&ollama_req).unwrap();
        assert!(
            json.get("tools").is_none(),
            "tools field should be omitted when None"
        );
    }

    #[test]
    fn test_to_ollama_request_assistant_with_tool_calls() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: None,
            messages: vec![LlmMessage {
                role: LlmRole::Assistant,
                content: LlmContent::Blocks(vec![
                    LlmContentBlock::Text("Let me search for that.".into()),
                    LlmContentBlock::ToolCall {
                        id: "call_1".into(),
                        name: "search".into(),
                        arguments: json!({"query": "weather"}),
                    },
                ]),
            }],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        assert_eq!(ollama_req.messages.len(), 1);

        let json = serde_json::to_value(&ollama_req).unwrap();
        let msg = &json["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "Let me search for that.");
        let tc = &msg["tool_calls"][0];
        assert_eq!(tc["function"]["name"], "search");
        assert_eq!(tc["function"]["arguments"]["query"], "weather");
    }

    #[test]
    fn test_to_ollama_request_tool_result_messages() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: None,
            messages: vec![LlmMessage {
                role: LlmRole::Tool,
                content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                    tool_call_id: "call_1".into(),
                    content: LlmToolResultContent::Text("Sunny, 22°C".into()),
                    is_error: false,
                }]),
            }],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        assert_eq!(ollama_req.messages.len(), 1);
        assert_eq!(ollama_req.messages[0].role, "tool");
        assert_eq!(ollama_req.messages[0].content, "Sunny, 22°C");
    }

    #[test]
    fn test_to_ollama_request_tool_result_blocks() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: None,
            messages: vec![LlmMessage {
                role: LlmRole::Tool,
                content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                    tool_call_id: "call_1".into(),
                    content: LlmToolResultContent::Blocks(vec![
                        LlmToolResultBlock::Text("Part 1. ".into()),
                        LlmToolResultBlock::Text("Part 2.".into()),
                    ]),
                    is_error: false,
                }]),
            }],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        assert_eq!(ollama_req.messages[0].content, "Part 1. Part 2.");
    }

    #[test]
    fn test_to_ollama_request_tool_result_text_content() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: None,
            messages: vec![LlmMessage {
                role: LlmRole::Tool,
                content: LlmContent::Text("raw tool output".into()),
            }],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        assert_eq!(ollama_req.messages[0].role, "tool");
        assert_eq!(ollama_req.messages[0].content, "raw tool output");
    }

    #[test]
    fn test_to_ollama_request_blocks_flatten_to_text() {
        let provider = make_provider();
        let req = LlmRequest {
            model: "llama3".into(),
            system: None,
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Blocks(vec![
                    LlmContentBlock::Text("Part one. ".into()),
                    LlmContentBlock::Text("Part two.".into()),
                ]),
            }],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        assert_eq!(ollama_req.messages[0].content, "Part one. Part two.");
    }

    // -- Response conversion --

    #[test]
    fn test_from_ollama_response_basic() {
        let resp = OllamaChatResponse {
            model: "llama3".into(),
            message: OllamaResponseMessage {
                role: "assistant".into(),
                content: "Hello! How can I help you?".into(),
                tool_calls: None,
            },
            done: true,
            total_duration: Some(1_500_000_000),
            eval_count: Some(15),
            prompt_eval_count: Some(42),
        };

        let llm = OllamaProvider::from_ollama_response(resp);
        assert_eq!(llm.text(), "Hello! How can I help you?");
        assert_eq!(llm.stop_reason, LlmStopReason::EndTurn);
        assert_eq!(llm.usage.input_tokens, 42);
        assert_eq!(llm.usage.output_tokens, 15);
        assert!(llm.reasoning.is_none());
    }

    #[test]
    fn test_from_ollama_response_missing_token_counts() {
        let resp = OllamaChatResponse {
            model: "llama3".into(),
            message: OllamaResponseMessage {
                role: "assistant".into(),
                content: "Response".into(),
                tool_calls: None,
            },
            done: true,
            total_duration: None,
            eval_count: None,
            prompt_eval_count: None,
        };

        let llm = OllamaProvider::from_ollama_response(resp);
        assert_eq!(llm.usage.input_tokens, 0);
        assert_eq!(llm.usage.output_tokens, 0);
    }

    #[test]
    fn test_from_ollama_response_think_block_extraction() {
        let resp = OllamaChatResponse {
            model: "deepseek-r1".into(),
            message: OllamaResponseMessage {
                role: "assistant".into(),
                content: "<think>\nLet me reason about this.\n</think>\n\nThe answer is 42.".into(),
                tool_calls: None,
            },
            done: true,
            total_duration: Some(2_000_000_000),
            eval_count: Some(30),
            prompt_eval_count: Some(50),
        };

        let llm = OllamaProvider::from_ollama_response(resp);
        assert_eq!(llm.text(), "The answer is 42.");
        assert_eq!(llm.reasoning, Some("Let me reason about this.".into()));
    }

    #[test]
    fn test_from_ollama_response_empty_content() {
        let resp = OllamaChatResponse {
            model: "llama3".into(),
            message: OllamaResponseMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: None,
            },
            done: true,
            total_duration: None,
            eval_count: None,
            prompt_eval_count: None,
        };

        let llm = OllamaProvider::from_ollama_response(resp);
        assert!(llm.content.is_empty());
        assert_eq!(llm.text(), "");
    }

    #[test]
    fn test_from_ollama_response_tool_calls() {
        let resp = OllamaChatResponse {
            model: "llama3".into(),
            message: OllamaResponseMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: Some(vec![OllamaToolCall {
                    function: OllamaFunctionCall {
                        name: "get_weather".into(),
                        arguments: json!({"city": "Tokyo"}),
                    },
                }]),
            },
            done: true,
            total_duration: Some(1_000_000_000),
            eval_count: Some(10),
            prompt_eval_count: Some(20),
        };

        let llm = OllamaProvider::from_ollama_response(resp);
        assert_eq!(llm.content.len(), 1);
        match &llm.content[0] {
            LlmResponseContent::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "ollama_tc_0");
                assert_eq!(name, "get_weather");
                assert_eq!(arguments, &json!({"city": "Tokyo"}));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn test_from_ollama_response_tool_calls_stop_reason() {
        let resp = OllamaChatResponse {
            model: "llama3".into(),
            message: OllamaResponseMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: Some(vec![OllamaToolCall {
                    function: OllamaFunctionCall {
                        name: "search".into(),
                        arguments: json!({"q": "test"}),
                    },
                }]),
            },
            done: true,
            total_duration: None,
            eval_count: None,
            prompt_eval_count: None,
        };

        let llm = OllamaProvider::from_ollama_response(resp);
        assert_eq!(llm.stop_reason, LlmStopReason::ToolUse);
    }

    #[test]
    fn test_from_ollama_response_tool_calls_with_text() {
        let resp = OllamaChatResponse {
            model: "llama3".into(),
            message: OllamaResponseMessage {
                role: "assistant".into(),
                content: "I'll look that up for you.".into(),
                tool_calls: Some(vec![OllamaToolCall {
                    function: OllamaFunctionCall {
                        name: "search".into(),
                        arguments: json!({"query": "rust"}),
                    },
                }]),
            },
            done: true,
            total_duration: None,
            eval_count: Some(25),
            prompt_eval_count: Some(30),
        };

        let llm = OllamaProvider::from_ollama_response(resp);
        assert_eq!(llm.content.len(), 2);
        assert!(
            matches!(&llm.content[0], LlmResponseContent::Text(t) if t == "I'll look that up for you.")
        );
        assert!(
            matches!(&llm.content[1], LlmResponseContent::ToolCall { name, .. } if name == "search")
        );
        assert_eq!(llm.stop_reason, LlmStopReason::ToolUse);
    }

    #[test]
    fn test_from_ollama_response_multiple_tool_calls() {
        let resp = OllamaChatResponse {
            model: "llama3".into(),
            message: OllamaResponseMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: Some(vec![
                    OllamaToolCall {
                        function: OllamaFunctionCall {
                            name: "search".into(),
                            arguments: json!({"q": "a"}),
                        },
                    },
                    OllamaToolCall {
                        function: OllamaFunctionCall {
                            name: "calculate".into(),
                            arguments: json!({"expr": "1+1"}),
                        },
                    },
                ]),
            },
            done: true,
            total_duration: None,
            eval_count: None,
            prompt_eval_count: None,
        };

        let llm = OllamaProvider::from_ollama_response(resp);
        assert_eq!(llm.content.len(), 2);
        match &llm.content[0] {
            LlmResponseContent::ToolCall { id, name, .. } => {
                assert_eq!(id, "ollama_tc_0");
                assert_eq!(name, "search");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        match &llm.content[1] {
            LlmResponseContent::ToolCall { id, name, .. } => {
                assert_eq!(id, "ollama_tc_1");
                assert_eq!(name, "calculate");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn test_from_ollama_response_empty_tool_calls_is_end_turn() {
        let resp = OllamaChatResponse {
            model: "llama3".into(),
            message: OllamaResponseMessage {
                role: "assistant".into(),
                content: "Just text.".into(),
                tool_calls: Some(vec![]),
            },
            done: true,
            total_duration: None,
            eval_count: None,
            prompt_eval_count: None,
        };

        let llm = OllamaProvider::from_ollama_response(resp);
        assert_eq!(llm.stop_reason, LlmStopReason::EndTurn);
    }

    // -- Error parsing --

    #[test]
    fn test_error_response_parsing() {
        let body = r#"{"error": "model 'nonexistent' not found"}"#;
        let err: OllamaErrorResponse = serde_json::from_str(body).unwrap();
        assert_eq!(err.error, "model 'nonexistent' not found");
    }

    // -- Trait method checks --

    #[test]
    fn test_provider_capabilities() {
        let provider = make_provider();
        assert_eq!(provider.provider_name(), "ollama");
        assert_eq!(provider.model_name(), "llama3");
        assert_eq!(provider.max_tokens(), 4096);
        assert!(provider.supports_tool_calling());
        assert!(!provider.supports_vision());
        assert!(!provider.supports_extended_thinking());
    }

    // -- Request serialization --

    #[test]
    fn test_ollama_request_serialization() {
        let req = OllamaChatRequest {
            model: "llama3".into(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: "Hello".into(),
                tool_calls: None,
            }],
            stream: false,
            options: OllamaOptions { num_predict: 4096 },
            tools: None,
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "llama3");
        assert_eq!(json["stream"], false);
        assert_eq!(json["options"]["num_predict"], 4096);
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "Hello");
        // Optional fields should be omitted when None
        assert!(json.get("tools").is_none());
        assert!(json["messages"][0].get("tool_calls").is_none());
    }

    // -- Payload dump tests (mika#1387) -------------------------------------

    /// Tests for `try_dump_payload` use the process-wide env var
    /// `MIKA_OLLAMA_DUMP_PAYLOAD` and the process-wide one-shot
    /// `PAYLOAD_DUMP_FIRED` flag — must run serially and reset the flag
    /// before each case.
    mod payload_dump {
        use super::super::*;
        use serial_test::serial;
        use tempfile::TempDir;

        const ENV: &str = "MIKA_OLLAMA_DUMP_PAYLOAD";

        struct EnvGuard {
            key: &'static str,
        }
        impl EnvGuard {
            fn set(key: &'static str, value: &str) -> Self {
                unsafe { std::env::set_var(key, value) };
                Self { key }
            }
        }
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                unsafe { std::env::remove_var(self.key) };
            }
        }

        #[test]
        #[serial]
        fn r7_basic_dump_writes_body_and_flips_flag() {
            reset_payload_dump_flag();
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("payload.json");
            let _guard = EnvGuard::set(ENV, path.to_str().unwrap());

            let body = r#"{"model":"mika","messages":[{"role":"user","content":"hello"}]}"#;
            try_dump_payload(body);

            let written = std::fs::read_to_string(&path).expect("file should be written");
            assert_eq!(written, body);
            assert!(
                PAYLOAD_DUMP_FIRED.load(Ordering::Relaxed),
                "one-shot flag should be set after successful dump"
            );
            // Cleanup for the next serial test.
            reset_payload_dump_flag();
        }

        #[test]
        #[serial]
        fn r8_one_shot_second_call_is_noop() {
            reset_payload_dump_flag();
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("payload.json");
            let _guard = EnvGuard::set(ENV, path.to_str().unwrap());

            try_dump_payload(r#"{"first":"call"}"#);
            // Second invocation must not overwrite — content remains the first call's body.
            try_dump_payload(r#"{"second":"call"}"#);

            let written = std::fs::read_to_string(&path).expect("file should be written");
            assert_eq!(written, r#"{"first":"call"}"#);
            reset_payload_dump_flag();
        }

        #[test]
        #[serial]
        fn r9_truncation_marker_when_body_exceeds_cap() {
            reset_payload_dump_flag();
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("payload.json");
            let _guard = EnvGuard::set(ENV, path.to_str().unwrap());

            // Body larger than the 256 KiB cap.
            let body = "x".repeat(PAYLOAD_DUMP_CAP_BYTES + 1024);
            try_dump_payload(&body);

            let written = std::fs::read_to_string(&path).expect("file should be written");
            // First PAYLOAD_DUMP_CAP_BYTES bytes preserved verbatim.
            assert_eq!(
                &written[..PAYLOAD_DUMP_CAP_BYTES],
                &body[..PAYLOAD_DUMP_CAP_BYTES]
            );
            // Truncation marker appended with both byte counts.
            assert!(
                written.contains("<!-- TRUNCATED at "),
                "expected truncation marker, got tail: {}",
                &written[written.len().saturating_sub(200)..]
            );
            assert!(
                written.contains(&format!(
                    "total payload was {} bytes",
                    PAYLOAD_DUMP_CAP_BYTES + 1024
                )),
                "marker should report the total payload length"
            );
            reset_payload_dump_flag();
        }

        #[test]
        #[serial]
        fn env_unset_is_noop() {
            reset_payload_dump_flag();
            // No env var set — `try_dump_payload` returns without touching anything.
            unsafe { std::env::remove_var(ENV) };
            try_dump_payload(r#"{"unused":true}"#);
            assert!(
                !PAYLOAD_DUMP_FIRED.load(Ordering::Relaxed),
                "flag must not flip when env is unset"
            );
        }

        #[test]
        #[serial]
        fn env_empty_is_noop() {
            reset_payload_dump_flag();
            let _guard = EnvGuard::set(ENV, "");
            try_dump_payload(r#"{"unused":true}"#);
            assert!(
                !PAYLOAD_DUMP_FIRED.load(Ordering::Relaxed),
                "flag must not flip when env value is empty"
            );
        }
    }
}
