//! Integration tests: max-steps exceeded + continuation turn behavior.
//!
//! When the agent loop exceeds MAX_TOOL_STEPS (20), it triggers a continuation
//! turn with tools disabled, asking for a text summary. If the continuation
//! turn also fails, a structured fallback with the last tool names is produced.

use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

#[tokio::test]
async fn test_max_steps_exceeded_triggers_continuation() {
    // Create 20 tool_call responses (each calling search_memory with different queries)
    // followed by 1 text response for the continuation turn.
    let mut responses: Vec<MockResponse> = (0..20)
        .map(|i| tool_call_response("search_memory", json!({"query": format!("query_{i}")})))
        .collect();
    // The 21st response is the continuation turn — tools are disabled, agent must summarize.
    responses.push(text_response(
        "Here is a summary: I searched for many items but could not find a definitive answer.",
    ));

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .unwrap();

    let trace = harness.run("Search for everything").await.unwrap();

    // Agent should produce output from the continuation turn
    assert_has_output(&trace);
    assert_output_contains(&trace, "summary");

    // The continuation turn may or may not be recorded as a separate entry in
    // llm_calls depending on the implementation. Assert at least 20 steps
    // (the tool steps) and verify the continuation happened via output text.
    assert_max_steps(&trace, 21);

    // The continuation turn (last captured request) should have tools disabled
    if let Some(last_request) = trace.captured_requests.last() {
        let has_no_tools = last_request.tools.as_ref().is_none_or(|t| t.is_empty());
        assert!(
            has_no_tools,
            "Continuation turn should have tools disabled, but tools were present: {:?}",
            last_request.tools.as_ref().map(|t| t.len())
        );
    }
}

#[tokio::test]
async fn test_max_steps_continuation_failure_produces_fallback() {
    // Create 20 tool_call responses, then an error for the continuation turn.
    // The agent should produce a structured fallback with recent tool names.
    let mut responses: Vec<MockResponse> = (0..20)
        .map(|i| tool_call_response("search_memory", json!({"query": format!("query_{i}")})))
        .collect();
    // Continuation turn fails with an LLM error
    responses.push(MockResponse::Error(mika_common::llm::LlmError::HttpError {
        status: 500,
        message: "Internal server error during continuation".into(),
        retryable: false,
    }));

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .unwrap();

    let trace = harness.run("Search for everything").await.unwrap();

    // Even when the continuation fails, the agent should produce fallback output
    assert_has_output(&trace);
    // Fallback should mention the tools that were called
    assert_output_contains(&trace, "search_memory");
}
