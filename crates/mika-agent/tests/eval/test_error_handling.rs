//! Integration tests: error handling scenarios.

use mika_common::llm::mock::*;

use super::assertions::*;
use super::harness::EvalHarness;

#[tokio::test]
async fn test_max_tokens_stop_produces_output() {
    // MaxTokens is unrecoverable — agent should still return whatever text was generated
    let harness = EvalHarness::builder()
        .responses(vec![max_tokens_response(
            "I was about to say something but ran out of",
        )])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Tell me a long story").await.unwrap();

    // MaxTokens responses should still produce output
    assert_has_output(&trace);
    assert_no_tools(&trace);
}

#[tokio::test]
async fn test_content_filter_produces_empty_or_fallback() {
    // ContentFilter is unrecoverable — agent should handle gracefully
    let harness = EvalHarness::builder()
        .responses(vec![content_filter_response()])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Something problematic").await.unwrap();

    // ContentFilter may produce no text or a fallback. At minimum, it shouldn't panic.
    assert_no_tools(&trace);
    assert_exact_steps(&trace, 1);
}

#[tokio::test]
async fn test_llm_error_propagates() {
    // An LLM API error should propagate as an error result
    let harness = EvalHarness::builder()
        .responses(vec![MockResponse::Error(
            mika_common::llm::LlmError::HttpError {
                status: 500,
                message: "Internal server error".into(),
                retryable: false,
            },
        )])
        .build()
        .await
        .unwrap();

    let result = harness.run("Hello").await;

    // The agent should propagate the LLM error
    assert!(result.is_err(), "Expected LLM error to propagate");
}
