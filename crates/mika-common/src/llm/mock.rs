//! Mock LLM provider for deterministic integration testing.
//!
//! `MockLlmProvider` implements `LlmProvider` with pre-configured response sequences,
//! enabling agent loop integration tests without network calls.
//!
//! # Example
//! ```rust,ignore
//! use mika_common::llm::mock::*;
//! use serde_json::json;
//!
//! let provider = MockLlmProvider::builder()
//!     .response(text_response("Hello!"))
//!     .response(tool_call_response("search_memory", json!({"query": "test"})))
//!     .response(text_response("Found it!"))
//!     .build();
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use super::LlmProvider;
use super::error::LlmError;
use super::types::*;

/// A pre-configured mock response — either a successful LLM response or an error.
pub enum MockResponse {
    /// A successful LLM response.
    Success(LlmResponse),
    /// An LLM error (rate limit, network failure, etc.).
    Error(LlmError),
}

/// Configuration for `MockLlmProvider` trait method return values.
pub struct MockProviderConfig {
    pub provider_name: String,
    pub model_name: String,
    pub max_tokens: u32,
    pub supports_tool_calling: bool,
    pub supports_vision: bool,
    pub supports_extended_thinking: bool,
    /// When set, `check_health()` returns this error instead of `Ok(())`.
    pub health_error: Option<LlmError>,
}

impl Default for MockProviderConfig {
    fn default() -> Self {
        Self {
            provider_name: "mock".to_string(),
            model_name: "mock-model".to_string(),
            max_tokens: 4096,
            supports_tool_calling: true,
            supports_vision: false,
            supports_extended_thinking: false,
            health_error: None,
        }
    }
}

/// Sequence-based mock LLM provider for deterministic testing.
///
/// Returns pre-configured responses in order. Panics when the sequence is exhausted
/// (fast failure for test debugging). Captures all received requests for post-run inspection.
pub struct MockLlmProvider {
    responses: Mutex<Vec<MockResponse>>,
    cursor: AtomicUsize,
    captured_requests: Arc<Mutex<Vec<LlmRequest>>>,
    config: MockProviderConfig,
}

impl MockLlmProvider {
    /// Create a new builder for configuring a `MockLlmProvider`.
    pub fn builder() -> MockLlmProviderBuilder {
        MockLlmProviderBuilder::default()
    }

    /// Get all requests captured during the test run.
    pub fn captured_requests(&self) -> Vec<LlmRequest> {
        self.captured_requests.lock().unwrap().clone()
    }

    /// Get the number of responses that have been consumed.
    pub fn calls_made(&self) -> usize {
        self.cursor.load(Ordering::SeqCst)
    }

    /// Replace the response sequence and reset the cursor.
    ///
    /// Useful when the test needs to seed DB data (which generates IDs)
    /// before configuring mock responses that reference those IDs.
    /// Must be called before `.run()` — not safe during concurrent access.
    pub fn clear_and_set(&self, responses: Vec<MockResponse>) {
        *self.responses.lock().unwrap() = responses;
        self.cursor.store(0, Ordering::SeqCst);
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        // Capture the request for post-run inspection
        self.captured_requests.lock().unwrap().push(request.clone());

        let index = self.cursor.fetch_add(1, Ordering::SeqCst);
        let responses = self.responses.lock().unwrap();
        if index >= responses.len() {
            panic!(
                "MockLlmProvider: exhausted all {} responses at call {}. \
                 Add more responses or verify agent behavior.",
                responses.len(),
                index + 1
            );
        }

        match &responses[index] {
            MockResponse::Success(resp) => Ok(resp.clone()),
            MockResponse::Error(err) => Err(err.clone()),
        }
    }

    fn provider_name(&self) -> &str {
        &self.config.provider_name
    }

    fn model_name(&self) -> &str {
        &self.config.model_name
    }

    fn max_tokens(&self) -> u32 {
        self.config.max_tokens
    }

    fn supports_tool_calling(&self) -> bool {
        self.config.supports_tool_calling
    }

    fn supports_vision(&self) -> bool {
        self.config.supports_vision
    }

    fn supports_extended_thinking(&self) -> bool {
        self.config.supports_extended_thinking
    }

    async fn check_health(&self) -> Result<(), LlmError> {
        match &self.config.health_error {
            Some(err) => Err(err.clone()),
            None => Ok(()),
        }
    }
}

/// Builder for `MockLlmProvider`.
#[derive(Default)]
pub struct MockLlmProviderBuilder {
    responses: Vec<MockResponse>,
    config: MockProviderConfig,
}

impl MockLlmProviderBuilder {
    /// Add a successful response to the sequence.
    pub fn response(mut self, response: MockResponse) -> Self {
        self.responses.push(response);
        self
    }

    /// Add multiple responses to the sequence.
    pub fn responses(mut self, responses: Vec<MockResponse>) -> Self {
        self.responses.extend(responses);
        self
    }

    /// Add an error response to the sequence.
    pub fn error(mut self, error: LlmError) -> Self {
        self.responses.push(MockResponse::Error(error));
        self
    }

    /// Set the provider name returned by `provider_name()`.
    pub fn provider_name(mut self, name: impl Into<String>) -> Self {
        self.config.provider_name = name.into();
        self
    }

    /// Set the model name returned by `model_name()`.
    pub fn model_name(mut self, name: impl Into<String>) -> Self {
        self.config.model_name = name.into();
        self
    }

    /// Set the max tokens returned by `max_tokens()`.
    pub fn max_tokens(mut self, max: u32) -> Self {
        self.config.max_tokens = max;
        self
    }

    /// Set whether tool calling is supported.
    pub fn supports_tool_calling(mut self, supports: bool) -> Self {
        self.config.supports_tool_calling = supports;
        self
    }

    /// Configure `check_health()` to return an error on every call.
    ///
    /// When set, `check_health()` returns `Err(error.clone())` instead of `Ok(())`.
    /// Useful for simulating degraded-provider scenarios in eval tests.
    pub fn health_error(mut self, error: LlmError) -> Self {
        self.config.health_error = Some(error);
        self
    }

    /// Build the `MockLlmProvider`.
    pub fn build(self) -> MockLlmProvider {
        MockLlmProvider {
            responses: Mutex::new(self.responses),
            cursor: AtomicUsize::new(0),
            captured_requests: Arc::new(Mutex::new(Vec::new())),
            config: self.config,
        }
    }
}

// -- Helper constructors for common response patterns --

/// Create a simple text response (EndTurn, no tool calls).
pub fn text_response(text: &str) -> MockResponse {
    MockResponse::Success(LlmResponse {
        content: vec![LlmResponseContent::Text(text.to_string())],
        reasoning: None,
        stop_reason: LlmStopReason::EndTurn,
        usage: LlmUsage::default(),
    })
}

/// Create a response with a single tool call (ToolUse stop reason).
pub fn tool_call_response(name: &str, args: Value) -> MockResponse {
    MockResponse::Success(LlmResponse {
        content: vec![LlmResponseContent::ToolCall {
            id: format!("tc_{}", uuid::Uuid::new_v4().as_simple()),
            name: name.to_string(),
            arguments: args,
        }],
        reasoning: None,
        stop_reason: LlmStopReason::ToolUse,
        usage: LlmUsage::default(),
    })
}

/// Create a response with multiple tool calls (ToolUse stop reason).
pub fn multi_tool_response(calls: Vec<(&str, Value)>) -> MockResponse {
    let content = calls
        .into_iter()
        .map(|(name, args)| LlmResponseContent::ToolCall {
            id: format!("tc_{}", uuid::Uuid::new_v4().as_simple()),
            name: name.to_string(),
            arguments: args,
        })
        .collect();

    MockResponse::Success(LlmResponse {
        content,
        reasoning: None,
        stop_reason: LlmStopReason::ToolUse,
        usage: LlmUsage::default(),
    })
}

/// Create a text response with extended thinking/reasoning.
pub fn thinking_response(text: &str, thinking: &str) -> MockResponse {
    MockResponse::Success(LlmResponse {
        content: vec![LlmResponseContent::Text(text.to_string())],
        reasoning: Some(thinking.to_string()),
        stop_reason: LlmStopReason::EndTurn,
        usage: LlmUsage::default(),
    })
}

/// Create a MaxTokens stop reason response (simulates hitting the token limit).
pub fn max_tokens_response(text: &str) -> MockResponse {
    MockResponse::Success(LlmResponse {
        content: vec![LlmResponseContent::Text(text.to_string())],
        reasoning: None,
        stop_reason: LlmStopReason::MaxTokens,
        usage: LlmUsage::default(),
    })
}

/// Create a ContentFilter stop reason response.
pub fn content_filter_response() -> MockResponse {
    MockResponse::Success(LlmResponse {
        content: vec![],
        reasoning: None,
        stop_reason: LlmStopReason::ContentFilter,
        usage: LlmUsage::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_text_response_helper() {
        let resp = text_response("Hello!");
        match resp {
            MockResponse::Success(r) => {
                assert_eq!(r.text(), "Hello!");
                assert_eq!(r.stop_reason, LlmStopReason::EndTurn);
                assert!(!r.has_tool_calls());
            }
            MockResponse::Error(_) => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_tool_call_response_helper() {
        let resp = tool_call_response("search_memory", json!({"query": "test"}));
        match resp {
            MockResponse::Success(r) => {
                assert_eq!(r.stop_reason, LlmStopReason::ToolUse);
                assert!(r.has_tool_calls());
                let calls = r.tool_calls();
                assert_eq!(calls.len(), 1);
                if let LlmResponseContent::ToolCall {
                    name, arguments, ..
                } = calls[0]
                {
                    assert_eq!(name, "search_memory");
                    assert_eq!(arguments, &json!({"query": "test"}));
                }
            }
            MockResponse::Error(_) => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_multi_tool_response_helper() {
        let resp = multi_tool_response(vec![
            ("search_memory", json!({"query": "a"})),
            ("store_fact", json!({"text": "b"})),
        ]);
        match resp {
            MockResponse::Success(r) => {
                assert_eq!(r.tool_calls().len(), 2);
            }
            MockResponse::Error(_) => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_thinking_response_helper() {
        let resp = thinking_response("Answer", "Let me think...");
        match resp {
            MockResponse::Success(r) => {
                assert_eq!(r.text(), "Answer");
                assert_eq!(r.reasoning.as_deref(), Some("Let me think..."));
            }
            MockResponse::Error(_) => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_max_tokens_response_helper() {
        let resp = max_tokens_response("partial");
        match resp {
            MockResponse::Success(r) => {
                assert_eq!(r.stop_reason, LlmStopReason::MaxTokens);
            }
            MockResponse::Error(_) => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_content_filter_response_helper() {
        let resp = content_filter_response();
        match resp {
            MockResponse::Success(r) => {
                assert_eq!(r.stop_reason, LlmStopReason::ContentFilter);
            }
            MockResponse::Error(_) => panic!("Expected Success"),
        }
    }

    #[tokio::test]
    async fn test_mock_provider_sequence() {
        let provider = MockLlmProvider::builder()
            .response(text_response("First"))
            .response(text_response("Second"))
            .build();

        let request = LlmRequest {
            model: "test".into(),
            system: None,
            messages: vec![],
            tools: None,
            max_tokens: 100,
            thinking: None,
        };

        let r1 = provider.send_message(&request).await.unwrap();
        assert_eq!(r1.text(), "First");

        let r2 = provider.send_message(&request).await.unwrap();
        assert_eq!(r2.text(), "Second");

        assert_eq!(provider.calls_made(), 2);
    }

    #[tokio::test]
    async fn test_mock_provider_captures_requests() {
        let provider = MockLlmProvider::builder()
            .response(text_response("OK"))
            .build();

        let request = LlmRequest {
            model: "test-model".into(),
            system: Some("system prompt".into()),
            messages: vec![],
            tools: None,
            max_tokens: 200,
            thinking: None,
        };

        let _ = provider.send_message(&request).await;
        let captured = provider.captured_requests();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].model, "test-model");
        assert_eq!(captured[0].system.as_deref(), Some("system prompt"));
    }

    #[tokio::test]
    #[should_panic(expected = "exhausted all 1 responses at call 2")]
    async fn test_mock_provider_panics_on_exhaustion() {
        let provider = MockLlmProvider::builder()
            .response(text_response("Only one"))
            .build();

        let request = LlmRequest {
            model: "test".into(),
            system: None,
            messages: vec![],
            tools: None,
            max_tokens: 100,
            thinking: None,
        };

        let _ = provider.send_message(&request).await;
        let _ = provider.send_message(&request).await; // Should panic
    }

    #[tokio::test]
    async fn test_mock_provider_error_injection() {
        let provider = MockLlmProvider::builder()
            .response(text_response("OK"))
            .error(LlmError::HttpError {
                status: 429,
                message: "rate limited".into(),
                retryable: true,
            })
            .build();

        let request = LlmRequest {
            model: "test".into(),
            system: None,
            messages: vec![],
            tools: None,
            max_tokens: 100,
            thinking: None,
        };

        let r1 = provider.send_message(&request).await;
        assert!(r1.is_ok());

        let r2 = provider.send_message(&request).await;
        assert!(r2.is_err());
        let err = r2.unwrap_err();
        assert!(err.is_retryable());
    }

    #[tokio::test]
    async fn test_mock_provider_trait_methods() {
        let provider = MockLlmProvider::builder()
            .provider_name("test-provider")
            .model_name("test-model-v1")
            .max_tokens(8192)
            .supports_tool_calling(false)
            .build();

        assert_eq!(provider.provider_name(), "test-provider");
        assert_eq!(provider.model_name(), "test-model-v1");
        assert_eq!(provider.max_tokens(), 8192);
        assert!(!provider.supports_tool_calling());
        assert!(!provider.supports_vision());
        assert!(!provider.supports_extended_thinking());
        assert!(provider.check_health().await.is_ok());
    }

    #[tokio::test]
    async fn test_mock_provider_health_error() {
        let provider = MockLlmProvider::builder()
            .health_error(LlmError::HttpError {
                status: 503,
                message: "service unavailable".into(),
                retryable: true,
            })
            .build();

        let result = provider.check_health().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            LlmError::HttpError { status, .. } => assert_eq!(status, 503),
            _ => panic!("Expected HttpError"),
        }
    }
}
