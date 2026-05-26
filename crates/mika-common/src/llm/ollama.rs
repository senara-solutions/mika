use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tracing::{Instrument, debug, info, info_span, warn};

use super::error::LlmError;
use super::openai::extract_think_block;
use super::types::*;
use super::{LlmProvider, RETRY_BUFFER_SECS, TYPICAL_CALL_DURATION_SECS};

// -- Ollama wire types --

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    num_predict: u32,
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
/// - Does not support tool calling or vision (deferred)
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    max_tokens: u32,
    #[cfg_attr(not(feature = "telemetry"), allow(dead_code))]
    log_llm_bodies: bool,
}

impl OllamaProvider {
    pub fn new(
        base_url: String,
        api_key: Option<String>,
        model: String,
        max_tokens: u32,
        log_llm_bodies: bool,
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
            log_llm_bodies,
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
            });
        }

        // Convert conversation messages
        for msg in &request.messages {
            match msg.role {
                LlmRole::User | LlmRole::Assistant => {
                    let role = match msg.role {
                        LlmRole::User => "user",
                        LlmRole::Assistant => "assistant",
                        _ => unreachable!(),
                    };
                    let content = match &msg.content {
                        LlmContent::Text(t) => t.clone(),
                        LlmContent::Blocks(blocks) => {
                            // Flatten blocks to text. Tool call/result blocks are
                            // unreachable in normal operation (the agent loop gates on
                            // supports_tool_calling() at agent.rs:2614). Log a warning
                            // as a contract-violation signal if encountered.
                            let mut has_non_text = false;
                            let text: String = blocks
                                .iter()
                                .filter_map(|b| match b {
                                    LlmContentBlock::Text(t) => Some(t.as_str()),
                                    _ => {
                                        has_non_text = true;
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("");
                            if has_non_text {
                                warn!(
                                    role = role,
                                    "ollama provider received non-text content blocks \
                                     (tool call/result/image) — these are unreachable in \
                                     normal operation; flattening to text only"
                                );
                            }
                            text
                        }
                    };
                    messages.push(OllamaMessage {
                        role: role.into(),
                        content,
                    });
                }
                LlmRole::Tool => {
                    // Tool result messages are unreachable when supports_tool_calling()
                    // returns false. Log warning and skip.
                    warn!(
                        "ollama provider received tool result message — \
                         this is unreachable in normal operation; skipping"
                    );
                }
            }
        }

        OllamaChatRequest {
            model: request.model.clone(),
            messages,
            stream: false,
            options: OllamaOptions {
                num_predict: self.max_tokens,
            },
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

        let content = if text.is_empty() {
            vec![]
        } else {
            vec![LlmResponseContent::Text(text)]
        };

        let usage = LlmUsage {
            input_tokens: response.prompt_eval_count.unwrap_or(0),
            output_tokens: response.eval_count.unwrap_or(0),
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };

        LlmResponse {
            content,
            reasoning,
            stop_reason: LlmStopReason::EndTurn,
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

        // Dev-mode body logging
        if tracing::enabled!(target: "mika::llm_debug", tracing::Level::DEBUG)
            && let Ok(body_json) = serde_json::to_string(request)
        {
            debug!(target: "mika::llm_debug", body = %body_json, provider = "ollama", "llm request body");
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
        "ollama"
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    fn supports_tool_calling(&self) -> bool {
        false
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
    fn test_to_ollama_request_tools_ignored() {
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
                description: "Search".into(),
                parameters: json!({"type": "object"}),
            }]),
            max_tokens: 4096,
            thinking: None,
        };

        let ollama_req = provider.to_ollama_request(&req);
        // Tools are not included in the ollama request format
        let json = serde_json::to_value(&ollama_req).unwrap();
        assert!(json.get("tools").is_none());
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
        assert!(!provider.supports_tool_calling());
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
            }],
            stream: false,
            options: OllamaOptions { num_predict: 4096 },
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "llama3");
        assert_eq!(json["stream"], false);
        assert_eq!(json["options"]["num_predict"], 4096);
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "Hello");
    }
}
