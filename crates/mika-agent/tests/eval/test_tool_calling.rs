//! Integration tests: tool calling scenarios.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use mika_agent::messaging::{MessageSender, SendOutcome};
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
    //
    // Note: uses search_memory instead of send_message because the send_message
    // turn boundary guard (#771) suppresses the second send_message call and
    // forces EndTurn, interfering with dedup testing.
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("search_memory", json!({"query": "sprint status"})),
                ("search_memory", json!({"query": "sprint status"})),
            ]),
            text_response("Sprint kicked off."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("start the sprint").await.unwrap();

    assert_has_output(&trace);
    // Two identical blocks should collapse to a single tool_calls row.
    let search_calls = trace.calls_for_tool("search_memory");
    assert_eq!(
        search_calls.len(),
        1,
        "Expected exactly 1 search_memory call, got {}: {:?}",
        search_calls.len(),
        search_calls.iter().map(|c| &c.input).collect::<Vec<_>>()
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
    // component and keys only on tool name. Two `search_memory` calls with
    // different queries must produce two tool_calls rows — they are legitimately
    // distinct invocations, not a provider artifact.
    //
    // Note: uses search_memory instead of send_message because the send_message
    // turn boundary guard (#771) suppresses the second send_message in the same
    // step. That suppression is intentional — a separate test validates it.
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("search_memory", json!({"query": "sprint alpha"})),
                ("search_memory", json!({"query": "sprint beta"})),
            ]),
            text_response("Found both."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("search for both sprints").await.unwrap();

    assert_has_output(&trace);
    let search_calls = trace.calls_for_tool("search_memory");
    assert_eq!(
        search_calls.len(),
        2,
        "Same tool with different args must not be deduplicated, got {} calls: {:?}",
        search_calls.len(),
        search_calls.iter().map(|c| &c.input).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_three_identical_tool_use_blocks_deduplicated() {
    // The dedup cache must collapse any N >= 2 identical blocks in a single
    // turn to exactly one execution. Exercises the cache-hit path twice.
    //
    // Note: uses search_memory instead of send_message because the send_message
    // turn boundary guard (#771) suppresses the second+ send_message calls and
    // forces EndTurn, which interferes with dedup testing.
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("search_memory", json!({"query": "ping"})),
                ("search_memory", json!({"query": "ping"})),
                ("search_memory", json!({"query": "ping"})),
            ]),
            text_response("Searched."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("ping").await.unwrap();

    assert_has_output(&trace);
    let search_calls = trace.calls_for_tool("search_memory");
    assert_eq!(
        search_calls.len(),
        1,
        "Three identical blocks must still collapse to one tool_calls row, got {}",
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

// --- prose-style tool call guard (#569) ---

#[tokio::test]
async fn test_prose_style_tool_call_retry() {
    // First response: LLM outputs a prose-style tool call as text instead of
    // using the structured API — e.g. `search_memory({"query": "meetings"})`.
    // The agent loop should detect this (the identifier matches a registered
    // tool) and re-prompt.
    // Second response: proper structured tool call.
    // Third response: text summary after tool execution.
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response("search_memory({\"query\": \"meetings\"})"),
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

#[tokio::test]
async fn test_prose_style_tool_call_unknown_tool_no_retry() {
    // The prose pattern uses an identifier that is NOT a registered tool.
    // The guard should NOT fire — the response passes through as normal text.
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "You can test it by running my_function({\"key\": \"value\"}).",
        )])
        .build()
        .await
        .unwrap();

    let trace = harness.run("How do I test this?").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "my_function");
    // 1 step: no retry — the identifier is not a registered tool
    assert_exact_steps(&trace, 1);
}

// --- send_message gateway error surfacing (#581) ---

/// Test sender returning a configurable outcome for eval harness tests.
struct EvalMockSender {
    outcome: SendOutcome,
}

#[async_trait]
impl MessageSender for EvalMockSender {
    async fn send(&self, _text: &str) -> Result<SendOutcome> {
        Ok(self.outcome.clone())
    }
}

#[tokio::test]
async fn test_send_message_gateway_failure_surfaces_error() {
    // LLM calls send_message, sender returns Failed -> tool_call should record success=false
    let sender = Arc::new(EvalMockSender {
        outcome: SendOutcome::Failed {
            reason: "gateway /send returned 502 Bad Gateway".to_string(),
        },
    });

    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("send_message", json!({"text": "Sprint started"})),
            text_response("I tried to send the message but delivery failed."),
        ])
        .message_sender(sender)
        .build()
        .await
        .unwrap();

    let trace = harness.run("Send a sprint update").await.unwrap();

    assert_tools_include(&trace, &["send_message"]);

    // The tool call should be recorded as a failure
    let send_calls = trace.calls_for_tool("send_message");
    assert_eq!(
        send_calls.len(),
        1,
        "expected exactly one send_message call"
    );
    assert!(
        !send_calls[0].success,
        "expected success=false for gateway failure, got success=true"
    );
    assert_tool_output_contains(&trace, "send_message", 0, "delivery failed");
    assert_tool_output_contains(&trace, "send_message", 0, "502 Bad Gateway");
}

#[tokio::test]
async fn test_send_message_gateway_success_records_success() {
    // LLM calls send_message, sender returns Delivered -> tool_call should record success=true
    let sender = Arc::new(EvalMockSender {
        outcome: SendOutcome::Delivered,
    });

    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("send_message", json!({"text": "Hello user"})),
            text_response("Message sent successfully."),
        ])
        .message_sender(sender)
        .build()
        .await
        .unwrap();

    let trace = harness.run("Send a hello message").await.unwrap();

    assert_tools_include(&trace, &["send_message"]);

    let send_calls = trace.calls_for_tool("send_message");
    assert_eq!(
        send_calls.len(),
        1,
        "expected exactly one send_message call"
    );
    assert!(
        send_calls[0].success,
        "expected success=true for delivered message, got success=false"
    );
    assert_tool_output_contains(&trace, "send_message", 0, "Message sent.");
}

#[tokio::test]
async fn test_send_message_no_channel_returns_success_with_redirect() {
    // LLM calls send_message, sender returns NoChannel (chat_id=0) ->
    // tool_call should record success=true with actionable redirect text (#650).
    let sender = Arc::new(EvalMockSender {
        outcome: SendOutcome::NoChannel,
    });

    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("send_message", json!({"text": "Build complete"})),
            text_response("I see there's no reply channel. I'll use run_gh instead."),
        ])
        .message_sender(sender)
        .build()
        .await
        .unwrap();

    let trace = harness.run("Notify about the build").await.unwrap();

    assert_tools_include(&trace, &["send_message"]);

    let send_calls = trace.calls_for_tool("send_message");
    assert_eq!(
        send_calls.len(),
        1,
        "expected exactly one send_message call"
    );
    // NoChannel is a success (not error) to prevent LLM retry loops
    assert!(
        send_calls[0].success,
        "expected success=true for NoChannel outcome, got success=false"
    );
    assert_tool_output_contains(&trace, "send_message", 0, "No reply channel");
    assert_tool_output_contains(&trace, "send_message", 0, "run_gh");
}

/// Regression test for mika#151: when the LLM returns `stop_reason: EndTurn`
/// with tool_use content blocks, the tool calls must be processed and their
/// summaries captured in the metadata. Previously these were silently dropped.
#[tokio::test]
async fn test_endturn_with_tool_use_blocks_processed() {
    // Step 0: normal ToolUse response with 2 tool calls
    // Step 1: EndTurn response that contains BOTH text AND tool_use blocks
    let harness = EvalHarness::builder()
        .responses(vec![
            multi_tool_response(vec![
                ("search_memory", json!({"query": "tasks"})),
                ("search_memory", json!({"query": "calendar"})),
            ]),
            endturn_with_tools_response(
                "Here is a summary of your tasks and calendar.",
                vec![(
                    "store_fact",
                    json!({"category": "event", "content": "User asked for summary"}),
                )],
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What do I have going on?").await.unwrap();

    // The text from the EndTurn response should be saved
    assert_has_output(&trace);
    assert_output_contains(&trace, "summary");

    // Tool calls from BOTH steps should be recorded
    assert_tools_include(&trace, &["search_memory", "store_fact"]);

    // Step 0: 2 search_memory calls, Step 1: 1 store_fact call = 3 total
    assert_eq!(
        trace.tool_calls.len(),
        3,
        "expected 3 tool calls (2 from ToolUse step + 1 from EndTurn-with-tool_use step), got {}",
        trace.tool_calls.len()
    );

    // The store_fact call should come from step 1 (the EndTurn step)
    let store_fact_calls = trace.calls_for_tool("store_fact");
    assert_eq!(store_fact_calls.len(), 1, "expected 1 store_fact call");
}

/// Regression test for mika#151: EndTurn with tool_use blocks and empty text
/// should still process the tool calls. In conversation mode, the agent loop
/// follows up on empty text with a re-prompt, so we provide a third response.
#[tokio::test]
async fn test_endturn_with_tool_use_blocks_and_empty_text() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("search_memory", json!({"query": "meetings"})),
            endturn_with_tools_response(
                "",
                vec![(
                    "store_fact",
                    json!({"category": "event", "content": "checked meetings"}),
                )],
            ),
            // The agent loop follows up on empty text in conversation mode
            text_response("I've checked your meetings and noted the results."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Check my meetings").await.unwrap();

    // Both tool calls from the first two steps should be recorded
    assert_tools_include(&trace, &["search_memory", "store_fact"]);
    assert_eq!(
        trace.tool_calls.len(),
        2,
        "expected 2 tool calls total, got {}",
        trace.tool_calls.len()
    );
}

/// Regression test for mika#151: EndTurn response with NO tool_use blocks
/// should behave exactly as before (regression guard).
#[tokio::test]
async fn test_endturn_without_tool_use_blocks_unchanged() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("search_memory", json!({"query": "priorities"})),
            text_response("Your top priority is the quarterly review."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("What should I focus on?").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "quarterly review");
    assert_tools_include(&trace, &["search_memory"]);
    assert_exact_steps(&trace, 2);
}

/// Regression test for mika#151: multiple tool_use blocks in a single
/// EndTurn response should all be processed.
#[tokio::test]
async fn test_endturn_with_multiple_tool_use_blocks() {
    let harness = EvalHarness::builder()
        .responses(vec![endturn_with_tools_response(
            "I've noted all your preferences.",
            vec![
                (
                    "store_fact",
                    json!({"category": "preference", "content": "likes coffee"}),
                ),
                (
                    "store_fact",
                    json!({"category": "preference", "content": "prefers morning meetings"}),
                ),
                (
                    "store_fact",
                    json!({"category": "preference", "content": "uses dark mode"}),
                ),
            ],
        )])
        .build()
        .await
        .unwrap();

    let trace = harness.run("Remember my preferences").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "preferences");

    let store_fact_calls = trace.calls_for_tool("store_fact");
    assert_eq!(
        store_fact_calls.len(),
        3,
        "expected all 3 store_fact calls from EndTurn-with-tool_use, got {}",
        store_fact_calls.len()
    );
}
