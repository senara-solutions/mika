//! Integration tests: multi-turn conversation persistence.
//!
//! Verifies that conversation history carries across multiple turns on the
//! same session, ensuring the LLM sees previous messages in context.

use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;

#[tokio::test]
async fn test_two_turn_conversation_carries_history() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Hello! How can I help you?")])
        .build()
        .await
        .unwrap();

    // Turn 1
    let trace1 = harness.run("Hello there").await.unwrap();
    assert_has_output(&trace1);
    assert_output_contains(&trace1, "Hello");

    // Turn 2 — use run_turn with fresh responses
    let trace2 = harness
        .run_turn(
            "Remember what I said earlier?",
            vec![text_response("Yes, you said 'Hello there'.")],
        )
        .await
        .unwrap();

    assert_has_output(&trace2);

    // Verify that the agent persisted both turns in the DB by checking
    // that the mock captured requests across both turns. Since clear_and_set
    // preserves captured_requests, we can see all requests.
    // The turn 2 LLM request (last in captured_requests) should contain
    // conversation history from turn 1 in its messages array.
    assert!(
        !trace2.captured_requests.is_empty(),
        "Expected at least 1 captured request in turn 2"
    );

    // The last captured request is from turn 2 — it should have more messages
    // than just the current turn's user message (history is loaded from DB).
    let turn2_request = trace2.captured_requests.last().unwrap();
    assert!(
        turn2_request.messages.len() >= 3,
        "Turn 2 request should contain history from turn 1 (user + assistant) plus \
         the turn 2 user message, got {} messages total",
        turn2_request.messages.len()
    );
}

#[tokio::test]
async fn test_three_turn_with_tool_call_carries_history() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("search_memory", json!({"query": "meetings"})),
            text_response("You have a meeting at 3pm."),
        ])
        .build()
        .await
        .unwrap();

    // Turn 1 — involves a tool call
    let trace1 = harness.run("What meetings do I have?").await.unwrap();
    assert_has_output(&trace1);
    assert_tools_include(&trace1, &["search_memory"]);

    // Turn 2
    let trace2 = harness
        .run_turn(
            "Tell me more about that meeting",
            vec![text_response("The 3pm meeting is about project planning.")],
        )
        .await
        .unwrap();

    assert_has_output(&trace2);

    // Turn 3
    let trace3 = harness
        .run_turn(
            "Should I prepare anything?",
            vec![text_response("You should prepare the quarterly report.")],
        )
        .await
        .unwrap();

    assert_has_output(&trace3);

    // Turn 3 should see all prior conversation history
    assert!(!trace3.captured_requests.is_empty());
    let turn3_request = trace3.captured_requests.last().unwrap();
    // Turn 3 request should contain history from turns 1 and 2
    // (multiple user + assistant messages + tool results from turn 1)
    assert!(
        turn3_request.messages.len() >= 5,
        "Turn 3 request should contain history from turns 1 and 2, got {} messages",
        turn3_request.messages.len()
    );
}
