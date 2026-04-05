//! Integration tests: tool calling scenarios.

use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

#[tokio::test]
async fn test_single_tool_call_then_response() {
    // Mock sequence: tool call → text response after tool result
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("search_memory", json!({"query": "meetings"})),
            text_response("I found some information about your meetings."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What meetings do I have?").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "meetings");
    assert_tools_include(&trace, &["search_memory"]);
    assert_exact_steps(&trace, 2); // 1 tool call + 1 final response
}

#[tokio::test]
async fn test_multiple_parallel_tool_calls() {
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("search_memory", json!({"query": "schedule"})),
                ("search_memory", json!({"query": "priorities"})),
            ]),
            text_response("Based on your schedule and priorities..."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What should I focus on today?").await.unwrap();

    assert_has_output(&trace);
    // Both search_memory calls should be recorded
    let search_calls = trace.calls_for_tool("search_memory");
    assert!(
        search_calls.len() >= 2,
        "Expected at least 2 search_memory calls, got {}",
        search_calls.len()
    );
}

#[tokio::test]
async fn test_tool_call_with_store_fact() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "store_fact",
                json!({
                    "category": "preference",
                    "text": "User prefers morning meetings"
                }),
            ),
            text_response("I've noted your preference for morning meetings."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("I prefer morning meetings").await.unwrap();

    assert_tools_include(&trace, &["store_fact"]);
    assert_has_output(&trace);
}

#[tokio::test]
async fn test_text_based_tool_call_retry() {
    // First response: LLM outputs XML tool call as text instead of structured API.
    // The agent loop should detect this and re-prompt.
    // Second response: proper structured tool call.
    // Third response: text summary after tool execution.
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response("<function=search_memory>\n{\"query\": \"meetings\"}\n</function>"),
            tool_call_response("search_memory", json!({"query": "meetings"})),
            text_response("Here are your meetings."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What meetings do I have?").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "meetings");
    assert_tools_include(&trace, &["search_memory"]);
    // 3 steps: text (retry), tool call, final response
    assert_exact_steps(&trace, 3);
}
