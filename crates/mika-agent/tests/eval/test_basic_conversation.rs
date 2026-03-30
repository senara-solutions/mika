//! Integration tests: basic conversation (EndTurn without tool calls).

use mika_common::llm::mock::*;

use super::assertions::*;
use super::harness::EvalHarness;

#[tokio::test]
async fn test_simple_text_response() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Hello! How can I help you today?")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Hi there").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Hello");
    assert_no_tools(&trace);
    assert_exact_steps(&trace, 1);
    assert_stop_reason(&trace, "EndTurn");
}

#[tokio::test]
async fn test_response_with_thinking() {
    let harness = EvalHarness::builder()
        .responses(vec![thinking_response(
            "The answer is 42.",
            "Let me think about the meaning of life...",
        )])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What is the meaning of life?").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "42");
    assert_no_tools(&trace);
    // Thinking is captured in the output
    assert!(trace.output.thinking.is_some());
}

#[tokio::test]
async fn test_system_prompt_contains_agent_identity() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("I'm Mika.")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Who are you?").await.unwrap();

    // System prompt should contain the agent's name/identity
    assert!(!trace.captured_requests.is_empty());
    // The system prompt is built from soul.md + identity + core memory.
    // At minimum it should exist and be non-empty.
    let system = trace.captured_requests[0].system.as_deref().unwrap_or("");
    assert!(!system.is_empty(), "System prompt should not be empty");
}

#[tokio::test]
async fn test_user_message_persisted_to_history() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Got it.")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Remember this message").await.unwrap();

    // The user message should appear in the captured request's message history
    assert!(!trace.captured_requests.is_empty());
    let messages = &trace.captured_requests[0].messages;
    assert!(
        !messages.is_empty(),
        "Request should contain at least the user message"
    );
}

#[tokio::test]
async fn test_tools_are_registered_in_request() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("OK")])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Hello").await.unwrap();

    // Default tools should be registered in the LLM request
    assert_tools_registered(&trace, &["search_memory", "store_fact"]);
}
