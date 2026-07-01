//! Scenario definitions for the multi-provider eval matrix.
//!
//! Re-exports promoted types from `src/calibration/scenario.rs` and provides
//! the provider-level scenario implementations.

use std::sync::Arc;
use std::time::Instant;

use mika_common::llm::LlmProvider;
use mika_common::llm::types::{LlmContent, LlmMessage, LlmRequest, LlmRole};

// Re-export promoted types
pub use mika_agent::calibration::scenario::{Scenario, ScenarioOutcome};

/// Registry of all available scenarios (test-local, not promoted).
pub struct ScenarioRegistry {
    pub scenarios: Vec<Scenario>,
}

impl ScenarioRegistry {
    /// Create the default registry with all built-in scenarios.
    pub fn default_scenarios() -> Self {
        Self {
            scenarios: vec![
                Scenario {
                    name: "basic_conversation",
                    description: "Simple text response with no tool calls",
                    run: |p| Box::pin(scenario_basic_conversation(p)),
                },
                Scenario {
                    name: "multi_turn_greeting",
                    description: "Two-turn conversation testing context carriage",
                    run: |p| Box::pin(scenario_multi_turn_greeting(p)),
                },
            ],
        }
    }
}

/// Scenario: Basic conversation — send a simple message, expect a text response.
async fn scenario_basic_conversation(provider: Arc<dyn LlmProvider>) -> ScenarioOutcome {
    let start = Instant::now();
    let provider_name = provider.provider_name().to_string();
    let model = provider.model_name().to_string();

    let request = LlmRequest {
        model: model.clone(),
        system: Some("You are a helpful assistant. Respond concisely.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text("Say 'hello world' and nothing else.".to_string()),
        }],
        tools: None,
        max_tokens: 100,
        thinking: None,
    };

    match provider.send_message(&request).await {
        Ok(response) => {
            let text = response.text().to_string();
            ScenarioOutcome {
                scenario: "basic_conversation".to_string(),
                provider: provider_name,
                model,
                success: !text.is_empty(),
                error: if text.is_empty() {
                    Some("Empty response".to_string())
                } else {
                    None
                },
                response_text: Some(text),
                input_tokens: Some(response.usage.input_tokens),
                output_tokens: Some(response.usage.output_tokens),
                latency_ms: start.elapsed().as_millis() as u64,
                emitted_shape: None,
            }
        }
        Err(e) => ScenarioOutcome {
            scenario: "basic_conversation".to_string(),
            provider: provider_name,
            model,
            success: false,
            error: Some(format!("{e}")),
            response_text: None,
            input_tokens: None,
            output_tokens: None,
            latency_ms: start.elapsed().as_millis() as u64,
            emitted_shape: None,
        },
    }
}

/// Scenario: Multi-turn greeting — two messages to test context persistence.
async fn scenario_multi_turn_greeting(provider: Arc<dyn LlmProvider>) -> ScenarioOutcome {
    let start = Instant::now();
    let provider_name = provider.provider_name().to_string();
    let model = provider.model_name().to_string();

    // Turn 1
    let request1 = LlmRequest {
        model: model.clone(),
        system: Some("You are a helpful assistant. Respond concisely.".to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text("My name is TestUser42.".to_string()),
        }],
        tools: None,
        max_tokens: 100,
        thinking: None,
    };

    let response1 = match provider.send_message(&request1).await {
        Ok(r) => r,
        Err(e) => {
            return ScenarioOutcome {
                scenario: "multi_turn_greeting".to_string(),
                provider: provider_name,
                model,
                success: false,
                error: Some(format!("Turn 1 failed: {e}")),
                response_text: None,
                input_tokens: None,
                output_tokens: None,
                latency_ms: start.elapsed().as_millis() as u64,
                emitted_shape: None,
            };
        }
    };

    // Turn 2 — ask about the name from Turn 1
    let request2 = LlmRequest {
        model: model.clone(),
        system: Some("You are a helpful assistant. Respond concisely.".to_string()),
        messages: vec![
            LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Text("My name is TestUser42.".to_string()),
            },
            LlmMessage {
                role: LlmRole::Assistant,
                content: LlmContent::Text(response1.text().to_string()),
            },
            LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Text("What is my name?".to_string()),
            },
        ],
        tools: None,
        max_tokens: 100,
        thinking: None,
    };

    match provider.send_message(&request2).await {
        Ok(response2) => {
            let text = response2.text().to_string();
            let has_name = text.to_lowercase().contains("testuser42");
            ScenarioOutcome {
                scenario: "multi_turn_greeting".to_string(),
                provider: provider_name,
                model,
                success: has_name,
                error: if !has_name {
                    Some(format!("Response did not contain 'TestUser42': {text}"))
                } else {
                    None
                },
                response_text: Some(text),
                input_tokens: Some(response2.usage.input_tokens),
                output_tokens: Some(response2.usage.output_tokens),
                latency_ms: start.elapsed().as_millis() as u64,
                emitted_shape: None,
            }
        }
        Err(e) => ScenarioOutcome {
            scenario: "multi_turn_greeting".to_string(),
            provider: provider_name,
            model,
            success: false,
            error: Some(format!("Turn 2 failed: {e}")),
            response_text: None,
            input_tokens: None,
            output_tokens: None,
            latency_ms: start.elapsed().as_millis() as u64,
            emitted_shape: None,
        },
    }
}
