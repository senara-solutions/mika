//! Integration tests: multi-step tool chains.

use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

#[tokio::test]
async fn test_two_step_tool_chain() {
    // Search → store → respond
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("search_memory", json!({"query": "priorities"})),
            tool_call_response(
                "store_fact",
                json!({"category": "commitment", "text": "Updated priority list"}),
            ),
            text_response("I've updated your priorities based on what I found."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Review and update my priorities")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_tool_order(&trace, &["search_memory", "store_fact"]);
    assert_exact_steps(&trace, 3);
}

#[tokio::test]
async fn test_three_step_chain_with_assertions() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("search_memory", json!({"query": "John's email"})),
            tool_call_response("search_memory", json!({"query": "meeting agenda"})),
            tool_call_response(
                "store_fact",
                json!({"category": "event", "text": "Meeting with John at 3pm"}),
            ),
            text_response("I've scheduled a meeting with John."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Find John's contact and schedule a meeting")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_max_steps(&trace, 4);
    // search_memory should come before store_fact
    assert_tool_order(&trace, &["search_memory", "store_fact"]);
}
