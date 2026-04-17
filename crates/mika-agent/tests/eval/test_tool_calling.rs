//! Integration tests: tool calling scenarios.

use mika_common::llm::mock::*;
use mika_common::llm::{LlmContent, LlmContentBlock, LlmRole};
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
async fn test_duplicate_tool_use_block_deduplicated() {
    // Regression test for #582: when the LLM emits two tool_use blocks with
    // identical (name, arguments) in a single response, the agent must
    // execute the underlying tool only once and persist only one tool_calls
    // row. The duplicate block still receives a tool_result (so the API
    // contract holds), but it reuses the cached output rather than
    // re-running the tool.
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("send_message", json!({"text": "sprint started"})),
                ("send_message", json!({"text": "sprint started"})),
            ]),
            text_response("Sprint kicked off."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("start the sprint").await.unwrap();

    assert_has_output(&trace);
    // Two identical blocks should collapse to a single tool_calls row.
    let send_calls = trace.calls_for_tool("send_message");
    assert_eq!(
        send_calls.len(),
        1,
        "Expected exactly 1 send_message call, got {}: {:?}",
        send_calls.len(),
        send_calls.iter().map(|c| &c.input).collect::<Vec<_>>()
    );

    // API contract: every tool_use id the LLM emitted must have a matching
    // tool_result block in the follow-up request, otherwise providers (notably
    // Anthropic) reject the conversation. Count tool_result blocks in the
    // second captured request (the one that carries the first turn's results).
    let second_request = trace
        .captured_requests
        .get(1)
        .expect("expected a follow-up LLM request after tool execution");
    let tool_result_count: usize = second_request
        .messages
        .iter()
        .filter(|m| m.role == LlmRole::Tool)
        .map(|m| match &m.content {
            LlmContent::Blocks(blocks) => blocks
                .iter()
                .filter(|b| matches!(b, LlmContentBlock::ToolResult { .. }))
                .count(),
            _ => 0,
        })
        .sum();
    assert_eq!(
        tool_result_count, 2,
        "Both tool_use ids (original + duplicate) must receive a tool_result to satisfy the API contract, got {tool_result_count}"
    );
}

#[tokio::test]
async fn test_same_tool_different_args_not_deduplicated() {
    // Guard against a regression where the dedup key drops the `arguments`
    // component and keys only on tool name. Two `send_message` calls with
    // different text must produce two tool_calls rows — they are legitimately
    // distinct invocations, not a provider artifact.
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("send_message", json!({"text": "sprint started"})),
                ("send_message", json!({"text": "sprint ended"})),
            ]),
            text_response("Sent both updates."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("send both updates").await.unwrap();

    assert_has_output(&trace);
    let send_calls = trace.calls_for_tool("send_message");
    assert_eq!(
        send_calls.len(),
        2,
        "Same tool with different args must not be deduplicated, got {} calls: {:?}",
        send_calls.len(),
        send_calls.iter().map(|c| &c.input).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_three_identical_tool_use_blocks_deduplicated() {
    // The dedup cache must collapse any N >= 2 identical blocks in a single
    // turn to exactly one execution. Exercises the cache-hit path twice.
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("send_message", json!({"text": "ping"})),
                ("send_message", json!({"text": "ping"})),
                ("send_message", json!({"text": "ping"})),
            ]),
            text_response("Pinged."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("ping").await.unwrap();

    assert_has_output(&trace);
    let send_calls = trace.calls_for_tool("send_message");
    assert_eq!(
        send_calls.len(),
        1,
        "Three identical blocks must still collapse to one tool_calls row, got {}",
        send_calls.len()
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
