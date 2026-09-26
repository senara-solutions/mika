//! Golden scenario: Send a message to another person.
//!
//! Class: ToolSelection | Expected tokens: 2000

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "tool_selection_send_message",
        GoldenScenarioMeta {
            class: ScenarioClass::ToolSelection,
            expected_tokens: 2000,
            description: "User asks to send a message; agent calls send_message tool",
        },
    );
}

#[tokio::test]
async fn test_tool_selection_send_message() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent calls send_message with the content
            tool_call_response(
                "send_message",
                json!({
                    "recipient": "John",
                    "message": "The meeting has been postponed. I'll send you the new time soon."
                }),
            ),
            // Step 2: Agent confirms delivery
            text_response(
                "I've sent the message to John letting him know the meeting is postponed.",
            ),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Send a message to John saying the meeting is postponed")
        .await
        .unwrap();

    // Hard assertions
    assert_tools_include(&trace, &["send_message"]);
    assert_has_output(&trace);
    assert_output_contains(&trace, "John");
}
